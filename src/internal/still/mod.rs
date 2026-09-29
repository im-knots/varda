//! Image decks: a still, raster or vector, drawn onto the deck.
//!
//! Raster files are decoded once. SVG is rasterized at the deck size and
//! redrawn when the deck is resized; see [`svg`].

pub mod svg;

use crate::renderer::GpuContext;
use crate::source::{
    AlphaPolicy, ControlError, ControlSpec, ControlValue, DeckSourceInstance, DeckSourceProvider,
    LibraryCreate, LibrarySection, ScaledBlit, ScalingMode, SourceConfig, SourceEnv, SourceFrame,
    SourceLoader, SourceQuery, decode_config, encode_config, scaling_mode_spec,
};
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::LazyLock;

pub const SOURCE_TYPE: &str = "Image";

/// File extensions the image source opens.
pub const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "tiff", "tga", "webp", "svg", "svgz",
];

static PARAMS: LazyLock<Vec<ControlSpec>> = LazyLock::new(|| vec![scaling_mode_spec()]);

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    path: String,
    #[serde(default)]
    scaling_mode: ScalingMode,
}

/// The image source type.
pub struct ImageProvider;

impl DeckSourceProvider for ImageProvider {
    fn id(&self) -> &'static str {
        SOURCE_TYPE
    }

    fn label(&self) -> &'static str {
        "Images"
    }

    fn icon(&self) -> &'static str {
        "🖼"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn library(&self, _query: &SourceQuery) -> LibrarySection {
        LibrarySection {
            create: Some(LibraryCreate::File {
                field: "path".into(),
                extensions: EXTENSIONS.iter().map(|e| (*e).to_string()).collect(),
                label: "Load image files as deck sources".into(),
            }),
            ..LibrarySection::default()
        }
    }

    fn loader(&self, config: &SourceConfig, _query: &SourceQuery) -> Option<Result<SourceLoader>> {
        Some(decode_config::<Config>(config).and_then(|config| {
            anyhow::ensure!(
                Path::new(&config.path).is_file(),
                "Image file not found: {}",
                config.path
            );
            let loader: SourceLoader = Box::new(move |gpu, width, height| {
                let mut image = Image::open(gpu, &config.path, width, height)?;
                image.blit.scaling_mode = config.scaling_mode;
                Ok(Box::new(image) as Box<dyn DeckSourceInstance>)
            });
            Ok(loader)
        }))
    }

    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        let loader = self
            .loader(config, &env.query())
            .context("image source has a loader")??;
        loader(env.gpu, env.width, env.height)
    }

    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        config.get("path").cloned().unwrap_or_default()
    }
}

/// One image deck.
pub struct Image {
    path: String,
    /// The uploaded image and its view; the texture keeps the view alive.
    upload: (wgpu::Texture, wgpu::TextureView),
    blit: ScaledBlit,
    /// Kept so the SVG can be re-rasterized at a new deck size. `None` for
    /// raster images.
    svg: Option<Box<usvg::Tree>>,
}

impl Image {
    /// Open an image file for a `width × height` deck.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read, decoded or rasterized, or the blit
    /// pipeline cannot be created.
    pub fn open(gpu: &GpuContext, path: &str, width: u32, height: u32) -> Result<Self> {
        let file = Path::new(path);
        let (rgba, svg) = if svg::is_svg_path(file) {
            let tree = svg::parse_file(file)?;
            (svg::rasterize(&tree, width, height)?, Some(Box::new(tree)))
        } else {
            let img = image::open(file)
                .with_context(|| format!("Failed to load image: {}", file.display()))?;
            (img.to_rgba8(), None)
        };
        let (texture, view) = upload_image_texture(gpu, &rgba);
        let blit = ScaledBlit::new(
            gpu,
            AlphaPolicy::Verbatim,
            "Image Blit Pass",
            rgba.dimensions(),
        )?;
        Ok(Self {
            path: path.to_string(),
            upload: (texture, view),
            blit,
            svg,
        })
    }

    /// A config for a deck of the image at `path`.
    pub fn config_for(path: &str) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                path: path.to_string(),
                scaling_mode: ScalingMode::default(),
            },
        )
    }

    /// Size of the texture the image was drawn into.
    pub fn source_size(&self) -> (u32, u32) {
        self.blit.source_size
    }

    /// Whether this deck holds vector artwork.
    pub fn is_vector(&self) -> bool {
        self.svg.is_some()
    }
}

impl DeckSourceInstance for Image {
    fn source_type(&self) -> &str {
        SOURCE_TYPE
    }

    fn label(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("image")
            .to_string()
    }

    fn config(&self) -> SourceConfig {
        encode_config(
            SOURCE_TYPE,
            &Config {
                path: self.path.clone(),
                scaling_mode: self.blit.scaling_mode,
            },
        )
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        self.blit.draw(frame, &self.upload.1);
        Ok(())
    }

    /// Re-render vector artwork at the new deck size. On failure the existing
    /// texture is kept, so the deck goes soft instead of black.
    fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        let Some(tree) = &self.svg else {
            return;
        };
        let size = svg::raster_size(tree, width, height);
        if size == self.blit.source_size {
            return;
        }
        let rgba = match svg::rasterize(tree, width, height) {
            Ok(rgba) => rgba,
            Err(e) => {
                log::warn!("Could not re-rasterize SVG deck '{}': {e}", self.path);
                return;
            }
        };
        self.upload = upload_image_texture(gpu, &rgba);
        self.blit.source_size = size;
    }

    fn patch(&mut self, config: &SourceConfig) {
        if let Ok(config) = config.decode::<Config>() {
            self.blit.scaling_mode = config.scaling_mode;
        }
    }

    fn control(&mut self, ctx: &mut crate::source::SourceControl) {
        self.blit.control(ctx);
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        self.blit.param(name)
    }

    fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        self.blit
            .set_param(name, value)
            .unwrap_or_else(|| Err(ControlError::Unknown(name.to_string())))
    }

    /// The image is blitted verbatim, so its alpha ignores the transparent flag.
    fn owns_alpha(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Upload straight-alpha RGBA pixels as a source texture.
pub fn upload_image_texture(
    gpu: &GpuContext,
    rgba: &image::RgbaImage,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (w, h) = rgba.dimensions();
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Image Source Texture"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * w),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 4:1 drawing, so stretching is distinguishable from fitting.
    const WIDE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 200 50" width="200" height="50">
        <rect width="200" height="50" fill="#3050ff"/></svg>"##;

    fn image_deck(gpu: &GpuContext, path: &Path, width: u32, height: u32) -> crate::deck::Deck {
        let image = Image::open(gpu, path.to_str().expect("utf-8 path"), width, height)
            .expect("image deck");
        crate::deck::Deck::from_source(gpu, Box::new(image), width, height)
    }

    fn source_size(deck: &crate::deck::Deck) -> (u32, u32) {
        crate::source::downcast_ref::<Image>(deck.source())
            .expect("an image deck")
            .blit
            .source_size
    }

    #[test]
    fn an_svg_deck_is_drawn_at_the_deck_size_not_the_files_own_size() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("art.svg");
        std::fs::write(&path, WIDE_SVG).expect("write svg");

        let deck = image_deck(&gpu, &path, 1920, 1080);
        // Rasterized at deck width, not the file's 200x50, and not stretched to 16:9.
        assert_eq!(source_size(&deck), (1920, 480));
    }

    #[test]
    fn changing_the_master_resolution_redraws_the_svg() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("art.svg");
        std::fs::write(&path, WIDE_SVG).expect("write svg");

        let mut deck = image_deck(&gpu, &path, 640, 360);
        assert_eq!(source_size(&deck), (640, 160));

        // Resizing to 4K redraws instead of magnifying the 640 px raster.
        deck.resize(&gpu, 3840, 2160);
        assert_eq!(source_size(&deck), (3840, 960));

        deck.resize(&gpu, 1280, 720);
        assert_eq!(source_size(&deck), (1280, 320));
    }

    #[test]
    fn a_raster_image_keeps_its_own_pixels_across_a_resize() {
        // A PNG has nothing to redraw, so resizing leaves the source alone.
        let Some(gpu) = crate::testing::headless_gpu() else {
            eprintln!("Skipping: no headless GPU available");
            return;
        };
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("art.png");
        image::RgbaImage::from_pixel(80, 40, image::Rgba([10, 20, 30, 255]))
            .save(&path)
            .expect("write png");

        let mut deck = image_deck(&gpu, &path, 640, 360);
        assert_eq!(source_size(&deck), (80, 40));
        deck.resize(&gpu, 3840, 2160);
        assert_eq!(source_size(&deck), (80, 40));
    }
}
