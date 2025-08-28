# icon-rust

Command line utility (Rust) for working with application icon files:

* Extract PNG images from existing `.ico` (Windows) / `.icns` (macOS) files.
* Build new `.ico` / `.icns` files either from a single high‑resolution source image or from a directory of images.

## Features

| Command | Purpose |
|---------|---------|
| `extract` | Split an `.ico` / `.icns` file into individual PNG frames (saved as `<width>x<height>-<index>.png`). |
| `build` | Generate an `.ico` / `.icns` from a single source image, auto‑resizing to common sizes. |
| `build-dir` | Generate an `.ico` / `.icns` using the largest image in a directory as the base (other files currently ignored). |

## Supported Sizes

Default embedded sizes:

* ICO: `16, 24, 32, 48, 64, 128, 256`
* ICNS: `16, 32, 64, 128, 256, 512, 1024` (+ Retina variants derived automatically by the format)

## Build & Install

```bash
cargo build --release
# Binary at target/release/icon-rust
```

Optionally install (adds to your Cargo bin path):

```bash
cargo install --path .
```

## Usage

Show help:

```bash
icon-rust --help
```

### 1. Extract icons

```bash
icon-rust extract path/to/app.ico output_dir
icon-rust extract path/to/app.icns output_dir
```

Result: PNG files like `16x16-0.png`, `32x32-1.png`, etc.

### 2. Build from a single image

```bash
icon-rust build base.png ico out/icon.ico
icon-rust build base.png icns out/icon.icns
```

Options:

* `--contain` (default `true`):
  * `true` (contain): Scale image to fit inside target square; transparent padding added.
  * `false` (cover): Scale to fully cover target square; central crop performed.

Example (cover mode):

```bash
icon-rust build logo.png ico out/logo.ico --contain=false
```

### 3. Build from a directory of images

```bash
icon-rust build-dir assets ico out/app.ico
icon-rust build-dir assets icns out/app.icns
```

Behavior:

* Scans `assets/` for `*.png`, `*.jpg`, `*.jpeg`.
* Attempts to parse a size from each filename (first number group, e.g. `icon-128.png`, `256.png`, `logo_64x64.png`).
* Currently uses the largest discovered image as a base and resizes it to all target sizes (future enhancement: pick per-size images when present).
* Uses `contain` scaling (padding) in this mode.

## Scaling Modes Explained

| Mode | When to Use | Result |
|------|-------------|--------|
| Contain | Preserve full artwork without cropping | Letterboxed / transparent padding possible |
| Cover | Fill entire square, accept edge cropping | No padding, possible crop |

## Exit Codes

* `0` success
* `>0` error (message printed to stderr)

## Examples

Extract Chromium icon:
```bash
icon-rust extract chromium.ico out/
```

Generate ICNS from a 1024×1024 PNG:
```bash
icon-rust build icon@1024.png icns dist/app.icns
```

Generate ICO (cover mode) from SVG exported PNG:
```bash
inkscape -o temp/icon.png -w 1024 -h 1024 icon.svg
icon-rust build temp/icon.png ico dist/app.ico --contain=false
```

Build from directory that contains multiple prepared sizes:
```bash
tree assets
# assets
# ├── 16.png
# ├── 32.png
# ├── 128.png
# └── 1024.png
icon-rust build-dir assets icns dist/app.icns
```

## Limitations / Notes

* ICNS extraction: only standard pixel sizes (16–1024) are probed; exotic icon types won't export.
* `build-dir` currently ignores intermediate size files beyond using the largest; enhancement pending.
* Only PNG/JPEG inputs supported (add formats by enabling more `image` crate features if needed).
* Alpha transparency preserved; no color profile transformations performed.
* No Windows `.exe` resource editing—only raw icon files.

## Roadmap Ideas

* Use per-size source images when available in `build-dir`.
* Optional JSON manifest input (define custom size set).
* Add WebP & SVG (via `resvg` or `usvg`) support.
* Parallelize resizing for performance.
* Provide a library API + optional Node.js (N-API) binding.

## Development

Run with backtraces during development:
```bash
RUST_BACKTRACE=1 cargo run -- extract sample.ico out
```

Format & lint:
```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
```

## License

MIT
