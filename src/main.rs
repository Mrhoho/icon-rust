use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
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

fn extract_ico(path: &Path, out_dir: &Path, debug: bool) -> Result<()> {
    #[derive(Debug, Clone)]
    struct DirEntry {
        width: u8,
        height: u8,
        bitcount: u16,
        bytes_in_res: u32,
        image_offset: u32,
    }
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut header = [0u8; 6];
    f.read_exact(&mut header)?;
    if u16::from_le_bytes([header[0], header[1]]) != 0 {
        bail!("Invalid ICO reserved");
    }
    if u16::from_le_bytes([header[2], header[3]]) != 1 {
        bail!("Not ICO");
    }
    let count = u16::from_le_bytes([header[4], header[5]]) as usize;
    let mut dir = vec![0u8; 16 * count];
    f.read_exact(&mut dir)?;
    let mut entries: Vec<DirEntry> = Vec::with_capacity(count);
    for i in 0..count {
        let o = i * 16;
        entries.push(DirEntry {
            width: dir[o],
            height: dir[o + 1],
            bitcount: u16::from_le_bytes([dir[o + 6], dir[o + 7]]),
            bytes_in_res: u32::from_le_bytes([dir[o + 8], dir[o + 9], dir[o + 10], dir[o + 11]]),
            image_offset: u32::from_le_bytes([dir[o + 12], dir[o + 13], dir[o + 14], dir[o + 15]]),
        });
    }
    // pick largest (treat 0 as 256); tie-break by bitcount then bytes
    let mut best = None;
    let mut best_key = (0u32, 0u16, 0u32); // (area, bitcount, bytes)
    for e in &entries {
        let w = if e.width == 0 { 256 } else { e.width as u32 };
        let h = if e.height == 0 { 256 } else { e.height as u32 };
        let area = w * h;
        let key = (area, e.bitcount, e.bytes_in_res);
        if key > best_key {
            best = Some(e.clone());
            best_key = key;
            if debug {
                eprintln!(
                    "[debug] new best candidate {}x{} bpp={} bytes={}",
                    w, h, e.bitcount, e.bytes_in_res
                );
            }
        }
    }
    let e = best.ok_or_else(|| anyhow!("No entries"))?;
    let w_decl = if e.width == 0 { 256 } else { e.width as u32 };
    let h_decl = if e.height == 0 { 256 } else { e.height as u32 };
    if debug {
        eprintln!(
            "[debug] chosen entry decl={}x{} bpp={} off={} bytes={} ",
            w_decl, h_decl, e.bitcount, e.image_offset, e.bytes_in_res
        );
    }
    f.seek(SeekFrom::Start(e.image_offset as u64))?;
    let mut blob = vec![0u8; e.bytes_in_res as usize];
    f.read_exact(&mut blob)?;
    ensure_dir(out_dir)?;
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if blob.len() >= 8 && &blob[..8] == PNG_SIG {
        // png
        let img = image::load_from_memory(&blob).with_context(|| "decode PNG")?;
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let out_path = out_dir.join(format!("{}x{}.png", w, h));
        rgba.save(&out_path)?;
        if debug {
            eprintln!("[debug] wrote {}", out_path.display());
        }
        return Ok(());
    }
    // DIB path minimal support (32bpp + 8bpp indexed)
    if blob.len() < 40 {
        bail!("Unsupported blob format");
    }
    let header_size = u32::from_le_bytes(blob[0..4].try_into().unwrap()) as usize;
    if header_size < 40 {
        bail!("Unsupported DIB header");
    }
    let dib_w = i32::from_le_bytes(blob[4..8].try_into().unwrap()) as u32;
    let dib_h_total = i32::from_le_bytes(blob[8..12].try_into().unwrap());
    if dib_h_total <= 0 {
        bail!("Invalid DIB height");
    }
    let dib_h = (dib_h_total as u32) / 2;
    let bpp = u16::from_le_bytes(blob[14..16].try_into().unwrap());
    let compression = u32::from_le_bytes(blob[16..20].try_into().unwrap());
    let clr_used = u32::from_le_bytes(blob[32..36].try_into().unwrap());
    if compression != 0 {
        bail!("Compressed DIB unsupported");
    }
    if bpp == 32 {
        let expected = (dib_w * dib_h) as usize * 4;
        if blob.len() < header_size + expected {
            bail!("Truncated 32bpp data");
        }
        let data = &blob[header_size..header_size + expected];
        let mut rgba = RgbaImage::new(dib_w, dib_h);
        for y in 0..dib_h {
            let src_row = (dib_h - 1 - y) as usize;
            for x in 0..dib_w {
                let i = (src_row * dib_w as usize + x as usize) * 4;
                let b = data[i];
                let g = data[i + 1];
                let r = data[i + 2];
                let a = data[i + 3];
                rgba.put_pixel(x, y, Rgba([r, g, b, a]));
            }
        }
        let out_path = out_dir.join(format!("{}x{}.png", dib_w, dib_h));
        rgba.save(&out_path)?;
        if debug {
            eprintln!("[debug] wrote {} (DIB32)", out_path.display());
        }
        return Ok(());
    }
    if bpp == 8 {
        let palette_len = if clr_used > 0 { clr_used as usize } else { 256 };
        let palette_bytes = palette_len * 4;
        if blob.len() < header_size + palette_bytes {
            bail!("Truncated palette");
        }
        let palette = &blob[header_size..header_size + palette_bytes];
        let row_stride = ((dib_w * bpp as u32 + 31) / 32) * 4;
        let pixel_array_size = (row_stride * dib_h) as usize;
        let pixel_offset = header_size + palette_bytes;
        if blob.len() < pixel_offset + pixel_array_size {
            bail!("Truncated pixel array");
        }
        let pixels = &blob[pixel_offset..pixel_offset + pixel_array_size];
        let mask_stride = ((dib_w + 31) / 32) * 4;
        let mask_offset = pixel_offset + pixel_array_size;
        let mask = if blob.len() >= mask_offset + (mask_stride * dib_h) as usize {
            Some(&blob[mask_offset..mask_offset + (mask_stride * dib_h) as usize])
        } else {
            None
        };
        let mut rgba = RgbaImage::new(dib_w, dib_h);
        for y in 0..dib_h {
            let src_row = (dib_h - 1 - y) as usize;
            let row_start = src_row * row_stride as usize;
            for x in 0..dib_w {
                let idx8 = pixels[row_start + x as usize] as usize;
                let base = (idx8.min(palette_len - 1)) * 4;
                let b = palette[base];
                let g = palette[base + 1];
                let r = palette[base + 2];
                rgba.put_pixel(x, y, Rgba([r, g, b, 0xFF]));
            }
        }
        if let Some(mask_bytes) = mask {
            for y in 0..dib_h {
                let src_row = (dib_h - 1 - y) as usize;
                let row_off = src_row * mask_stride as usize;
                for x in 0..dib_w {
                    let byte_index = row_off + (x / 8) as usize;
                    let bit = 7 - (x % 8);
                    if byte_index < mask_bytes.len() && ((mask_bytes[byte_index] >> bit) & 1) == 1 {
                        rgba.get_pixel_mut(x, y).0[3] = 0;
                    }
                }
            }
        }
        let out_path = out_dir.join(format!("{}x{}.png", dib_w, dib_h));
        rgba.save(&out_path)?;
        if debug {
            eprintln!("[debug] wrote {} (DIB8)", out_path.display());
        }
        return Ok(());
    }
    bail!("Unsupported DIB bpp={}", bpp)
}

// Attempt to manually decode a PNG-backed ICO entry when ico crate fails (e.g., indexed color PNG)
// Legacy stub kept for compatibility (no longer used)
#[allow(dead_code)]
fn try_decode_entry_png(
    _path: &Path,
    _entry: &ico::IconDirEntry,
    _debug: bool,
) -> Result<Option<ico::IconImage>> {
    Ok(None)
}

// Removed multi-image write helper; simplified single largest extraction.

fn extract_icns(path: &Path, out_dir: &Path, debug: bool) -> Result<()> {
    use icns::{IconFamily, IconType};
    let mut data = Vec::new();
    File::open(path)?.read_to_end(&mut data)?;
    let family = IconFamily::read(data.as_slice()).with_context(|| "read icns")?;
    let mut best_img: Option<(u32, u32, icns::Image)> = None;
    let sizes = [16u32, 32, 64, 128, 256, 512, 1024];
    for s in sizes {
        if let Some(t) = IconType::from_pixel_size(s, s) {
            if let Ok(img) = family.get_icon_with_type(t) {
                let w = img.width();
                let h = img.height();
                if debug {
                    eprintln!("[debug] candidate {}x{}", w, h);
                }
                let area = w * h;
                if best_img.as_ref().map(|(bw, bh, _)| bw * bh).unwrap_or(0) < area {
                    best_img = Some((w, h, img));
                }
            }
        }
    }
    let (w, h, img) = best_img.ok_or_else(|| anyhow!("No images in ICNS"))?;
    ensure_dir(out_dir)?;
    let out_path = out_dir.join(format!("{}x{}.png", w, h));
    image::RgbaImage::from_raw(w, h, img.data().to_vec())
        .ok_or_else(|| anyhow!("raw to image"))?
        .save(&out_path)?;
    if debug {
        eprintln!("[debug] wrote {}", out_path.display());
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
    Extract {
        input: PathBuf,
        out_dir: PathBuf,
        #[clap(long)]
        debug: bool,
    },
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
        Commands::Extract {
            input,
            out_dir,
            debug,
        } => {
            let ext = input
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext.as_str() {
                "ico" => extract_ico(&input, &out_dir, debug)?,
                "icns" => extract_icns(&input, &out_dir, debug)?,
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
