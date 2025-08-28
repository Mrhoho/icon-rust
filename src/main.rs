use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use image::{DynamicImage, GenericImageView, Rgba, RgbaImage, imageops, imageops::FilterType};

// ============ Shared helpers ============

fn resize_contain(img: &DynamicImage, size: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let scale = (size as f32 / w as f32).min(size as f32 / h as f32);
    let nw = (w as f32 * scale).round().max(1.0) as u32;
    let nh = (h as f32 * scale).round().max(1.0) as u32;
    let resized = img.resize(nw, nh, FilterType::Lanczos3).to_rgba8();
    let mut canvas = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    let dx = ((size as i64 - nw as i64) / 2).max(0);
    let dy = ((size as i64 - nh as i64) / 2).max(0);
    imageops::replace(&mut canvas, &resized, dx, dy);
    canvas
}

fn resize_cover(img: &DynamicImage, size: u32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let scale = (size as f32 / w as f32).max(size as f32 / h as f32);
    let nw = (w as f32 * scale).round().max(size as f32) as u32;
    let nh = (h as f32 * scale).round().max(size as f32) as u32;
    let resized = img.resize(nw, nh, FilterType::Lanczos3);
    let rx = ((resized.width() - size) / 2).min(resized.width() - 1);
    let ry = ((resized.height() - size) / 2).min(resized.height() - 1);
    imageops::crop_imm(&resized, rx, ry, size, size).to_image()
}

fn resized_rgba(base: &DynamicImage, size: u32, contain: bool) -> RgbaImage {
    if contain {
        resize_contain(base, size)
    } else {
        resize_cover(base, size)
    }
}

fn load_image(path: &Path) -> Result<DynamicImage> {
    image::open(path).with_context(|| format!("Open image {}", path.display()))
}

fn ensure_dir(path: &Path) -> Result<()> {
    if path.exists() && !path.is_dir() {
        bail!("{} exists and is not dir", path.display());
    }
    fs::create_dir_all(path).with_context(|| format!("create dir {}", path.display()))
}

// ============ ICO / ICNS build ============

fn build_ico(source: &DynamicImage, contain: bool, out: &Path) -> Result<()> {
    use ico::{IconDir, IconDirEntry, IconImage, ResourceType};
    let sizes: &[u32] = &[16, 24, 32, 48, 64, 128, 256];
    let mut dir = IconDir::new(ResourceType::Icon);
    for &s in sizes {
        let rgba = resized_rgba(source, s, contain);
        let (w, h) = rgba.dimensions();
        let icon = IconImage::from_rgba_data(w, h, rgba.into_raw());
        let entry = IconDirEntry::encode(&icon).with_context(|| format!("encode {}px", s))?;
        dir.add_entry(entry);
    }
    if let Some(parent) = out.parent() {
        ensure_dir(parent)?;
    }
    let mut f = File::create(out).with_context(|| format!("create {}", out.display()))?;
    dir.write(&mut f)
        .with_context(|| format!("write ico {}", out.display()))
}

fn build_icns(source: &DynamicImage, contain: bool, out: &Path) -> Result<()> {
    use icns::{IconFamily, IconType, Image, PixelFormat};
    use std::collections::BTreeSet;
    let all_sizes: &[u32] = &[16, 32, 64, 128, 256, 512, 1024, 32, 64, 256, 512, 1024];
    let sizes: BTreeSet<u32> = all_sizes.iter().cloned().collect();
    let mut family = IconFamily::new();
    for s in sizes {
        if let Some(icon_type) = IconType::from_pixel_size(s, s) {
            let rgba = resized_rgba(source, s, contain);
            let (w, h) = rgba.dimensions();
            let data = rgba.into_raw();
            let img = Image::from_data(PixelFormat::RGBA, w, h, data)
                .with_context(|| format!("img {}px", s))?;
            family
                .add_icon_with_type(&img, icon_type)
                .with_context(|| format!("add {}", s))?;
        }
    }
    if let Some(parent) = out.parent() {
        ensure_dir(parent)?;
    }
    let mut f = File::create(out).with_context(|| format!("create {}", out.display()))?;
    family
        .write(&mut f)
        .with_context(|| format!("write icns {}", out.display()))
}

// Build from a directory of images (various sizes)
fn build_from_dir(dir: &Path, format: TargetFormat, out: &Path) -> Result<()> {
    // Map size->path: choose best (exact size) or pick largest for scaling down later.
    let mut size_map: Vec<(u32, PathBuf)> = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("read dir {}", dir.display()))? {
        let entry = entry?;
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
            match ext.to_ascii_lowercase().as_str() {
                "png" | "jpg" | "jpeg" => {}
                _ => continue,
            };
        } else {
            continue;
        }
        // Extract size from filename like 16.png or icon-32x32.png etc.
        let fname = p.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let mut parsed: Option<u32> = None;
        for token in fname.split(|c: char| !c.is_ascii_digit()) {
            if token.len() > 0 {
                if let Ok(v) = token.parse::<u32>() {
                    if v > 0 {
                        parsed = Some(v);
                        break;
                    }
                }
            }
        }
        if let Some(sz) = parsed {
            size_map.push((sz, p));
        }
    }
    if size_map.is_empty() {
        bail!("No sized images found in {}", dir.display());
    }
    // We'll pick a base largest image to scale others if needed.
    size_map.sort_by_key(|(s, _)| *s);
    let largest = size_map.last().unwrap().1.clone();
    let largest_img = load_image(&largest)?;
    let contain = true; // directory mode assumes contain for padding
    match format {
        TargetFormat::Ico => build_ico(&largest_img, contain, out),
        TargetFormat::Icns => build_icns(&largest_img, contain, out),
    }
}

// ============ Extract ============

fn extract_ico(path: &Path, out_dir: &Path) -> Result<()> {
    use ico::IconDir;
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let dir = IconDir::read(&mut f).with_context(|| format!("read ico {}", path.display()))?;
    ensure_dir(out_dir)?;
    for (idx, entry) in dir.entries().iter().enumerate() {
        let img = entry
            .decode()
            .with_context(|| format!("decode entry {}", idx))?;
        let w = img.width();
        let h = img.height();
        let mut out_path = out_dir.join(format!("{}x{}-{}.png", w, h, idx));
        // guarantee unique name
        let mut counter = 1;
        while out_path.exists() {
            out_path = out_dir.join(format!("{}x{}-{}-{}.png", w, h, idx, counter));
            counter += 1;
        }
        image::RgbaImage::from_raw(w, h, img.rgba_data().to_vec())
            .ok_or_else(|| anyhow!("raw to image"))?
            .save(&out_path)
            .with_context(|| format!("save {}", out_path.display()))?;
    }
    Ok(())
}

fn extract_icns(path: &Path, out_dir: &Path) -> Result<()> {
    use icns::{IconFamily, IconType};
    let mut data = Vec::new();
    File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .read_to_end(&mut data)?;
    let family = IconFamily::read(data.as_slice())
        .with_context(|| format!("read icns {}", path.display()))?;
    ensure_dir(out_dir)?;
    // Iterate known types to try decode each.
    let mut idx = 0usize;
    let probe_sizes = [16u32, 32, 64, 128, 256, 512, 1024];
    for s in probe_sizes {
        if let Some(icon_type) = IconType::from_pixel_size(s, s) {
            if let Ok(img) = family.get_icon_with_type(icon_type) {
                let w = img.width();
                let h = img.height();
                let mut out_path = out_dir.join(format!("{}x{}-{}.png", w, h, idx));
                let mut counter = 1;
                while out_path.exists() {
                    out_path = out_dir.join(format!("{}x{}-{}-{}.png", w, h, idx, counter));
                    counter += 1;
                }
                image::RgbaImage::from_raw(w, h, img.data().to_vec())
                    .ok_or_else(|| anyhow!("raw to image"))?
                    .save(&out_path)
                    .with_context(|| format!("save {}", out_path.display()))?;
                idx += 1;
            }
        }
    }
    if idx == 0 {
        bail!("No icons decoded from {}", path.display());
    }
    Ok(())
}

// ============ CLI ============

#[derive(Copy, Clone, Debug, ValueEnum)]
enum TargetFormat {
    Ico,
    Icns,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Extract all frames/images from an .ico or .icns into PNG files
    Extract { input: PathBuf, out_dir: PathBuf },
    /// Build icon (.ico/.icns) from a single base image (auto-resize)
    Build {
        input: PathBuf,
        #[clap(value_enum)]
        format: TargetFormat,
        output: PathBuf,
        #[clap(long, default_value_t = true)]
        contain: bool,
    },
    /// Build from a directory of images (largest used as base)
    BuildDir {
        dir: PathBuf,
        #[clap(value_enum)]
        format: TargetFormat,
        output: PathBuf,
    },
}

#[derive(Parser, Debug)]
#[command(version, about = "Icon utility: extract/build ICO/ICNS", long_about=None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Extract { input, out_dir } => {
            let ext = input
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext.as_str() {
                "ico" => extract_ico(&input, &out_dir)?,
                "icns" => extract_icns(&input, &out_dir)?,
                _ => bail!("Unsupported input extension: {}", ext),
            }
        }
        Commands::Build {
            input,
            format,
            output,
            contain,
        } => {
            let img = load_image(&input)?;
            match format {
                TargetFormat::Ico => build_ico(&img, contain, &output)?,
                TargetFormat::Icns => build_icns(&img, contain, &output)?,
            }
        }
        Commands::BuildDir {
            dir,
            format,
            output,
        } => {
            build_from_dir(&dir, format, &output)?;
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {:#}", e);
        std::process::exit(1);
    }
}
