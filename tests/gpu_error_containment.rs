//! A GPU error must not end the performance.
//!
//! wgpu's default validation-error callback panics, which on the render thread
//! kills the app. Shaders are user input, so a bad one must be survivable.

use varda::renderer::GpuContext;

mod common;
use common::headless_gpu as headless;

/// Provoke a real validation error: a texture row stride smaller than the
/// texture's actual row.
fn provoke_validation_error(gpu: &GpuContext) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("containment test target"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // 16 px of RGBA8 is 64 bytes per row; claim 16 and wgpu rejects it.
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &vec![0u8; 16 * 16 * 4],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(16),
            rows_per_image: Some(16),
        },
        wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
    );
}

#[test]
fn a_validation_error_is_captured_instead_of_aborting() {
    let Some(gpu) = headless() else {
        return;
    };

    // With wgpu's default handler this call panics and the process dies.
    provoke_validation_error(&gpu);

    assert!(
        gpu.errors.fault_count() > 0,
        "the error handler did not observe the fault — is it still installed?"
    );
    let faults = gpu.errors.take_faults();
    assert!(!faults.is_empty(), "the fault was counted but not retained");
    assert!(
        faults[0].message.to_lowercase().contains("bytes per row"),
        "unexpected fault message: {}",
        faults[0].message
    );
}

#[test]
fn the_device_still_works_after_a_validation_error() {
    let Some(gpu) = headless() else {
        return;
    };
    provoke_validation_error(&gpu);
    let _ = gpu.errors.take_faults();

    // Quarantining one deck is only sound if a validation error drops the
    // command without losing the device.
    let deck = varda::deck::Deck::solid_color(&gpu, [0.2, 0.4, 0.6, 1.0], 32, 32);
    let audio = varda::audio::AudioData::default();
    let modulation = varda::modulation::ModulationEngine::new();
    let mut deck = deck;
    let mut cmds = Vec::new();
    deck.render(&gpu, &audio, &modulation, 0, &mut cmds)
        .expect("render succeeds after a validation error");
    gpu.queue.submit(cmds);
    let _ = gpu.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(5)),
    });

    assert!(
        deck.gpu_error().is_none(),
        "an unrelated earlier fault must not quarantine a healthy deck"
    );
}

#[test]
fn faults_are_attributed_to_the_deck_that_caused_them() {
    let Some(gpu) = headless() else {
        return;
    };
    // Errors raised outside any deck carry no context...
    provoke_validation_error(&gpu);
    let unscoped = gpu.errors.take_faults();
    assert_eq!(unscoped.len(), 1);
    assert_eq!(unscoped[0].context, None);

    // ...and errors raised inside one name it, so the renderer disables the
    // right deck.
    {
        let _scope = gpu.errors.scope("deck deadbeef");
        provoke_validation_error(&gpu);
    }
    let scoped = gpu.errors.take_faults();
    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].context.as_deref(), Some("deck deadbeef"));
}
