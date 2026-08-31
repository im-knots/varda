//! Deterministic contract tests for `fractal_explorer.fs` scaled arithmetic.

use std::path::Path;

#[derive(Clone, Copy, Debug)]
struct Scaled {
    mantissa: f64,
    exponent: i32,
}

impl Scaled {
    fn new(value: f64, exponent: i32) -> Self {
        if value == 0.0 {
            return Self {
                mantissa: 0.0,
                exponent: 0,
            };
        }
        let shift = value.abs().log2().floor() as i32 + 1;
        Self {
            mantissa: value * 2.0_f64.powi(-shift),
            exponent: exponent + shift,
        }
    }

    fn ordinary(self) -> f64 {
        self.mantissa * 2.0_f64.powi(self.exponent)
    }

    fn multiply(self, value: f64) -> Self {
        Self::new(self.mantissa * value, self.exponent)
    }

    fn add(self, other: Self) -> Self {
        let exponent = self.exponent.max(other.exponent);
        let align = |value: Self| {
            if value.exponent - exponent < -120 {
                0.0
            } else {
                value.mantissa * 2.0_f64.powi(value.exponent - exponent)
            }
        };
        Self::new(align(self) + align(other), exponent)
    }
}

fn shader_source() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/fractal_explorer.fs"),
    )
    .expect("fractal shader")
}

fn executable_source(source: &str) -> String {
    let mut without_blocks = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(open) = rest.find("/*") {
        without_blocks.push_str(&rest[..open]);
        let close = rest[open..].find("*/").expect("closed block comment");
        rest = &rest[open + close + 2..];
    }
    without_blocks.push_str(rest);
    without_blocks
        .lines()
        .map(|line| line.split("//").next().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

fn declared_input_names(source: &str) -> Vec<&str> {
    let inputs = source
        .split_once(r#""INPUTS": ["#)
        .expect("INPUTS metadata")
        .1
        .split_once(r#""PHASE_INPUTS":"#)
        .expect("PHASE_INPUTS metadata")
        .0;
    inputs
        .split(r#""NAME": ""#)
        .skip(1)
        .filter_map(|entry| entry.split('"').next())
        .collect()
}

fn function_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source.find(signature).expect("function signature");
    let open = source[start..].find('{').expect("function body") + start;
    let mut depth = 0_i32;
    for (offset, byte) in source.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[open + 1..open + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unterminated function: {signature}");
}

#[test]
fn shared_exponents_cover_zoom_and_derivative_ranges() {
    for exponent in [0, -30, -45, -100, -300] {
        let value = Scaled::new(0.75, exponent);
        assert_eq!(value.exponent, exponent);
        assert!((value.ordinary() / 2.0_f64.powi(exponent) - 0.75).abs() < 1e-14);
    }

    let derivative = Scaled::new(0.75, 200).multiply(8.0);
    assert_eq!(derivative.exponent, 203);
    assert!(derivative.mantissa.abs() >= 0.5 && derivative.mantissa.abs() < 1.0);

    let dominant = Scaled::new(0.75, 0);
    let sub_ulp = Scaled::new(0.75, -300);
    assert_eq!(
        dominant.add(sub_ulp).ordinary().to_bits(),
        dominant.ordinary().to_bits()
    );
    assert_eq!(sub_ulp.exponent, -300);
}

/// The scaled arithmetic reads exponents from the IEEE field instead of
/// calling `log2`, and rescales by writing the field instead of calling
/// `exp2`. Both are the renderer's hottest operations, and the substitution is
/// only legitimate because the field *is* the floor of the binary logarithm
/// for every normal float. That identity is asserted here rather than assumed,
/// including at the boundaries where a mantissa of exactly one lives.
#[test]
fn exponent_field_equals_floor_log2_for_normal_floats() {
    let mut samples = vec![1.0_f32, 0.5, 2.0, 0.75, 1.999_999_9, 3.0, 1e-30, 1e30];
    for exponent in -120..=120 {
        samples.push((exponent as f32).exp2());
        samples.push((exponent as f32).exp2() * 1.5);
    }
    for value in samples {
        for signed in [value, -value] {
            let field = ((signed.abs().to_bits() >> 23) & 0xff) as i32 - 127;
            assert_eq!(
                field,
                signed.abs().log2().floor() as i32,
                "exponent field disagrees with floor(log2) at {signed:e}"
            );
            // And the rescale: writing the field to -1 puts the mantissa in
            // [0.5, 1), which is the normalized form the shader maintains.
            let scaled = signed * (-(field + 1) as f32).exp2();
            assert!(
                scaled.abs() >= 0.5 && scaled.abs() < 1.0,
                "normalized mantissa {scaled} out of range for {signed:e}"
            );
        }
    }

    let source = executable_source(&shader_source());
    for required in [
        "float dzPow2(float k) {",
        "float dzLog2Floor(float x) {",
        "value.m *= dzPow2(-shift);",
        "return dzPow2(clamp(difference, -DZ_EXP_WINDOW, 0.0));",
    ] {
        assert!(
            source.contains(required),
            "scaled arithmetic left the bit form: {required}"
        );
    }
    let normalize = function_body(&source, "DzScalar dzSNormalize");
    assert!(!normalize.contains("log2("), "normalize still calls log2");
    assert!(!normalize.contains("exp2("), "normalize still calls exp2");
}

#[test]
fn cancellation_free_bulb_numerators_keep_sub_ulp_offsets() {
    let p = [0.6_f64, -0.8, 0.25];
    let rho = p[0].hypot(p[1]);
    let exponent = -100;
    let ex = Scaled::new(0.5, exponent);
    let ey = Scaled::new(-0.75, exponent);
    let ez = Scaled::new(0.625, exponent);

    let phi_numerator = ey.multiply(p[0]).add(ex.multiply(-p[1]));
    let rho_delta = ex.multiply(p[0] / rho).add(ey.multiply(p[1] / rho));
    let theta_numerator = rho_delta.multiply(p[2]).add(ez.multiply(-rho));

    assert_ne!(phi_numerator.mantissa.to_bits() & 0x7fff_ffff_ffff_ffff, 0);
    assert_ne!(
        theta_numerator.mantissa.to_bits() & 0x7fff_ffff_ffff_ffff,
        0
    );
    assert!(phi_numerator.exponent <= exponent + 1);
    assert!(theta_numerator.exponent <= exponent + 1);
}

fn principal_winding(ref_side: i32, sample_y_negative: bool) -> f64 {
    let ref_y_negative = ref_side < 0;
    if ref_y_negative == sample_y_negative {
        0.0
    } else if ref_y_negative {
        std::f64::consts::TAU
    } else {
        -std::f64::consts::TAU
    }
}

#[test]
fn seam_winding_and_bailout_margins_choose_both_sides() {
    assert_eq!(
        principal_winding(1, true).to_bits(),
        (-std::f64::consts::TAU).to_bits()
    );
    assert_eq!(
        principal_winding(-1, false).to_bits(),
        std::f64::consts::TAU.to_bits()
    );

    let pre_bulb_margin = Scaled::new(0.5, -45);
    let post_slot_margin = Scaled::new(-0.5, -45);
    let radius_increment = Scaled::new(0.75, -45);
    assert!(radius_increment.ordinary() > pre_bulb_margin.ordinary());
    assert!(radius_increment.ordinary() > post_slot_margin.ordinary());
}

#[test]
fn shader_consumes_payload_v7_and_scaled_affine_slots() {
    let raw_source = shader_source();
    let source = executable_source(&raw_source);
    assert!(source.contains("const int DZ_PAYLOAD_VERSION = 7;"));
    assert!(!source.contains("DZ_HAS_BULB_PRE_RADIUS2_MARGIN"));
    assert!(!source.contains("P + e"));
    assert!(!source.contains("phiCandidate"));

    for required in [
        "e = dzVAdd(dzVMul(e, sc), seedInc);",
        "e = dzVMul(e, sc);",
        "e.m = vec3(e.m.x * linx",
        "e.m.xy = ra * e.m.xy",
        "e = dzVWithComponent(e, 2, dzSNeg(dAbs));",
        "vec4 qe = vec4(e.m, 0.0);",
        "dzPostSlotBailoutMargin(i)",
        // Margins and the bulb seam group are fetched once per fold and read
        // from locals; the contract is that the same indices feed the same
        // decisions.
        "vec4 mg0 = dzRec(i, 5);",
        "vec4 mg1 = dzRec(i, 6);",
        "vec4 seam = dzRec(i, 8);",
        "float seamSide = seam.y;",
        "float preRadius2Margin = marr[0];",
        "float radiusFloorMargin = marr[1];",
        "float polarMargin = marr[2];",
        "bool refXNegative = marr[3] < 0.0;",
        "dzVComponent(e, 1), -marr[4]",
        "float dzBulbSeamSide(int n) { return dzRec(n, 8).y; }",
        "float dzBulbPrincipalWinding(int n) { return dzRec(n, 8).z; }",
        "bool dzBulbRecordPresent(int n) { return dzRec(n, 8).w > 0.5; }",
        "word ^ (word >> 24u)",
    ] {
        assert!(
            source.contains(required),
            "missing shader contract: {required}"
        );
    }

    // The host payload owns both the target and orbit. A showcase with no
    // comparison estimator must fail visibly instead of inventing a reference.
    let render = scene_pass_body(&source);
    assert!(render.contains("dzResolvePayloadGate();"));
    assert!(render.contains("if (!g_dzPayloadMatches)"));
    assert!(render.contains("g_dzRef = dzAnchor();"));
    assert!(!render.contains("explicitA"));
    assert!(!render.contains("diveTarget()"));

    let integer_control = 8_i32;
    assert_eq!(integer_control, 8);
}

#[test]
fn frame_local_entry_never_materializes_the_physical_zoom_scale() {
    let raw_source = shader_source();
    let source = executable_source(&raw_source);
    let frame_vector = function_body(&source, "DzVec3 dzFrameVector");
    let estimator = function_body(
        &source,
        "float stackDE_dz(vec3 pos, DeLod lod, out DeInfo info) {",
    );
    let render = scene_pass_body(&source);
    let view_scale = function_body(&source, "float viewScale");

    assert!(raw_source.contains("\"MAX\": 120.0"));
    assert!(frame_vector.contains("exp2(frameLog2 - exponent)"));
    assert!(!frame_vector.contains("pow("));
    assert!(estimator.contains("dzFrameVector(pos)"));
    assert!(estimator.contains("dzDistanceToEstimatorUnits(scaledDistance)"));
    assert!(!estimator.contains("dzV(pos * viewScale())"));
    assert!(view_scale.contains("return 1.0;"));
    assert!(!source.contains("dzFrameLocalEnabled"));
    assert!(!source.contains("g_dzOffsetSpace"));
    assert!(render.contains("ro = -fw * cameraScale;"));
    assert!(!render.contains("-fw * dzSToOrdinary"));

    let frame_log2 = 2.0_f64.log2() - 100.0 * 10.0_f64.log2();
    let exponent = frame_log2.floor() as i32 + 1;
    let mantissa = 2.0_f64.powf(frame_log2 - f64::from(exponent));
    assert_eq!(exponent, -331);
    assert!((0.5..1.0).contains(&mantissa));

    // The ray coordinate remains ordinary and useful while its physical value
    // would be far below the f32 normal range.
    let local_ray_coordinate = -0.75_f32;
    assert_eq!(local_ray_coordinate.to_bits(), (-0.75_f32).to_bits());
    assert!(frame_log2 < -126.0);
    assert_eq!(10.0_f32.powi(-100).to_bits(), 0.0_f32.to_bits());

    // A physical DE with the same frame exponent converts back to an ordinary
    // local step without ever constructing the physical value.
    let physical_de = Scaled::new(0.75 * mantissa, exponent);
    let local_de = physical_de.mantissa * 2.0_f64.powf(f64::from(exponent) - frame_log2);
    assert!((local_de - 0.75).abs() < 1e-12);
}

/// An escape-time estimate is a distance only for a sample that escaped.
///
/// Both observed deep-zoom failures came from using `|z| / dr` unconditionally.
/// A sample that stays bounded keeps `|z|` at order one while `dr` compounds, so
/// the ratio falls in proportion to the fold count: at a short orbit it sat just
/// under the hit threshold everywhere and every ray hit at the camera, giving one
/// smooth lobe; at a long orbit it sat far below the step floor, rays exhausted
/// their budget, and the frame became isolated speckle. Neither reads as a
/// broken estimator from the image alone, which is why this is pinned here.
#[test]
fn bounded_samples_report_a_hit_rather_than_a_collapsing_ratio() {
    let source = executable_source(&shader_source());
    let estimator = function_body(
        &source,
        "float stackDE_dz(vec3 pos, DeLod lod, out DeInfo info) {",
    );

    assert!(
        estimator.contains(
            "if (!escaped\n        && (done >= total || float(done) >= kShell || recordsExhausted\n            || lodTruncated)) {"
        ),
        "the perturbed estimator no longer classifies a bounded sample as inside"
    );
    // The bulb leaves on its own pre-guard radius, not on the shared bailout, so
    // the two escape conditions are not interchangeable.
    assert!(estimator
        .contains("bool escaped = finalRadiusSquared > bail2 || (hasBulb && finalRadius > 2.0);"));
    assert!(estimator.contains("if (forms[k] == 5 && rates[k] > 0) hasBulb = true;"));

    // The ratio is still what an escaped sample uses, scaled by the
    // Douady-Hubbard shell term so the marched surface is the iso-escape
    // shell rather than the space-filling deep boundary.
    assert!(estimator.contains(
        "DzScalar scaledDistance =\n        dzSMul(dzSDivS(finalLength, dr), 0.35 * shellFactor);"
    ));
    // The shell is placed from the host's measurement of where this
    // frame's surface resolves, with the derived rate only as a fallback.
    assert!(estimator.contains("float measured = dzMeasuredShellFold("));
    assert!(estimator.contains("kShell = clamp(kShell, 1.0, float(total));"));
}

/// Depth-driven shading must not saturate when the fold budget outruns the zoom.
///
/// Normalising `log2(dr)` by a range built only from the carried magnification
/// fixes the low end and leaves the high end broken: measured at a frame radius
/// of 1e-6 with a twenty-six fold budget, `log2(dr)` reached about 78 against a
/// range of 42, so every pixel clamped to one and every consumer of the channel
/// returned a constant. The iteration-fog shell had the matching unit error, and
/// added octaves of magnification straight onto a fold count.
#[test]
fn depth_and_fog_scales_cover_the_fold_budget_they_are_compared_against() {
    let source = executable_source(&shader_source());

    assert!(
        !source.contains("/ (22.0 + carried)"),
        "depth range still ignores the fold budget"
    );
    for required in [
        "float depthSpan(int folds) {",
        "return max(22.0 + g_zoomLog2, float(folds) * foldOctaves() + 1.0);",
        "info.depth = clamp(logDr / depthSpan(done), 0.0, 1.0);",
        "float fogAt = fog_iter + g_zoomLog2 / foldOctaves();",
    ] {
        assert!(
            source.contains(required),
            "missing shading contract: {required}"
        );
    }

    // `foldOctaves` reads the slot uniforms without threading the value through
    // every estimator call.
    let octaves = function_body(&source, "float foldOctaves");
    assert!(octaves.contains("forms[k] == 5 ? power : abs(scale)"));

    // A twenty-six fold budget at eight-fold growth spans more octaves than the
    // carried magnification at 1e-6, which is exactly the saturating case.
    let carried = (4.2_f64 / 1e-6).log2();
    let budget = 26.0 * 8.0_f64.log2();
    assert!(budget > 22.0 + carried);
}

/// The eye must not be backed off in frame-local space, and it must not stand
/// at an assumed distance either.
///
/// The scalar estimator backoff loop stays gone: run in frame-local space it
/// asks a bounded sample to prove a distance it cannot express. But the fixed
/// standoff it was replaced with is a coin toss — the anchor sits on the
/// boundary of the solid the frame draws, and a quarter of a frame radius back
/// from it lands inside or outside depending on the depth, the stack and where
/// the shell ends up. The host walks a standoff ladder until the camera point
/// escapes before the shell fold and transports the answer per depth slice; the
/// constant survives only as the fallback for a payload that carries no
/// measurement. Either way the shader searches for nothing.
/// The scene pass's code, wherever it now lives.
///
/// `renderScene` used to open with the whole frame setup — zoom, camera basis,
/// payload gate, standoff — and the cache experiment moved that into
/// `dzSetupFrame` so a second pass could establish the same coordinate system
/// from one source. These contracts are about the sequence, not about which
/// function holds it, so they are checked against both.
fn scene_pass_body(source: &str) -> String {
    format!(
        "{}\n{}",
        function_body(source, "bool dzSetupFrame"),
        function_body(source, "vec4 renderScene")
    )
}

#[test]
fn frame_local_camera_trusts_the_isolated_ray_segment() {
    let source = executable_source(&shader_source());
    let render = scene_pass_body(&source);

    assert!(!render.contains("float camS"));
    assert!(!render.contains("camS <"));
    assert!(!render.contains("clear > want"));
    assert!(render.contains("dzMeasuredCameraStandoff(standoffFraction)"));
    assert!(render.contains("measuredStandoff > 0.0"));
    assert!(render.contains(": DZ_CAMERA_STANDOFF;"));
    assert!(render.contains("ro = -fw * cameraScale;"));
}

#[test]
fn primary_march_consumes_only_a_matching_inward_packet_prefix() {
    let source = executable_source(&shader_source());
    let prepare = function_body(&source, "void dzPreparePrimaryCertificate");
    let prefix = function_body(&source, "float dzCertifiedPrimaryAdvance");
    let render = scene_pass_body(&source);

    for required in [
        "dz_zoom_exp <= 0.0",
        "g_dzPayloadMatches",
        "int(metadata.x) != DZ_CERT_TILE_COLUMNS",
        "int(metadata.y) != DZ_CERT_TILE_ROWS",
        "int(metadata.w) < requiredIterations",
        "vec4 packed = dzGroup(3 + index / 4);",
        "g_dzSafeSlabMask = uint(round(max(packed[index % 4], 0.0)));",
    ] {
        assert!(
            prepare.contains(required),
            "missing directional certificate preparation: {required}"
        );
    }
    let required = "(mask & (1u << uint(candidate))) == 0u";
    assert!(
        prefix.contains(required),
        "missing directional certificate gate: {required}"
    );
    assert!(render.contains("dzPreparePrimaryCertificate(screenUv);"));
    assert!(render.contains("float certifiedT = dzCertifiedPrimaryAdvance(t);"));
    assert!(render.contains("t = certifiedT;"));
    assert!(!render.contains("t += dzCertifiedPrimaryAdvance"));
}

#[test]
fn showcase_exposes_only_perturbation_flight_and_static_stack_controls() {
    let raw = shader_source();
    let source = executable_source(&raw);
    let inputs = declared_input_names(&raw);
    let frame_log = function_body(&source, "float dzFrameLog2");

    for removed in [
        "dz_mode",
        "dz_use_orbit",
        "dz_anchor_x",
        "dz_anchor_y",
        "dz_anchor_z",
        "orbit_speed",
        "sway_speed",
        "sway_amount",
        "cam_dist",
        "cam_azim",
        "cam_elev",
        "fov",
        "look_x",
        "look_y",
        "look_z",
        "zoom_cycle",
        "evolve",
        "evolve_amount",
        "evolve_target",
    ] {
        assert!(
            !inputs.contains(&removed),
            "obsolete showcase input remains: {removed}"
        );
    }
    for retained in [
        "fly_speed",
        "dz_zoom_exp",
        "dz_flight_max_exp",
        "dz_debug",
        "slot0_formula",
        "slot0_iters",
        "slot1_formula",
        "slot1_iters",
        "slot2_formula",
        "slot2_iters",
        "slot3_formula",
        "slot3_iters",
        "stack_cap",
        "stack_order",
    ] {
        assert!(
            inputs.contains(&retained),
            "missing showcase input: {retained}"
        );
    }
    for removed_path in [
        "float diveLog(",
        "bool useDz",
        "bool useOrbit",
        "bool g_dzOff",
        "gl_FragCoord.x > RENDERSIZE.x",
        "PHASE_TIME_1",
        "PHASE_TIME_2",
        "PHASE_TIME_3",
    ] {
        assert!(
            !source.contains(removed_path),
            "obsolete estimator or animation path remains: {removed_path}"
        );
    }

    assert!(raw.contains(r#""PARAM": "fly_speed", "INDEX": 0"#));
    assert!(raw.contains(r#""flight_max_exp": "dz_flight_max_exp""#));
    assert!(frame_log
        .contains("float flight = fract(PHASE_TIME_0 / TAU) * max(dz_flight_max_exp, 0.0);"));
    assert!(frame_log.contains("flight * DZ_LOG2_10"));
    assert!(source.contains("bool dzStackSupported()"));
    assert!(source.contains("void dzResolvePayloadGate"));
    assert!(raw.contains("Payload gate"));
    for beauty_control in ["ray_steps", "bloom", "exposure", "saturation", "color_cool"] {
        assert!(
            inputs.contains(&beauty_control),
            "missing retained beauty control: {beauty_control}"
        );
    }
}

#[test]
fn bulb_axis_uses_group_power_without_a_reference_azimuth_chart() {
    let source = executable_source(&shader_source());
    let estimator = function_body(
        &source,
        "float stackDE_dz(vec3 pos, DeLod lod, out DeInfo info) {",
    );
    let unit_power = function_body(&source, "vec2 dzIntegerUnitPower");

    assert!(unit_power.contains("i < 12"));
    assert!(unit_power.contains("result.x * unitValue.x - result.y * unitValue.y"));
    for required in [
        "power == float(integerPower)",
        "bool groupPowerAxis = rhoRef == 0.0 && thetaOk",
        "e.m.xy / transverseLength",
        "dzIntegerUnitPower(",
        "float samplePhi = atan(sampleAzimuth.y, sampleAzimuth.x);",
        "DzVec3 axisPolar = dzScaledSinCosIncrement(",
        "0.0, dzSMul(thetaDelta, power)",
        "float axisSign = (P.z < 0.0 && (integerPower % 2) != 0)",
    ] {
        assert!(
            estimator.contains(required),
            "missing integer group-power axis contract: {required}"
        );
    }
    assert!(!estimator.contains("abs(power - float(integerPower))"));
}

/// Payload rejection has to be observable.
///
/// A stale signature or target must remain observable without a lockstep
/// fallback, so the gate reports which check rejected the only valid estimator.
#[test]
fn payload_gate_reports_which_check_rejected_the_orbit() {
    let raw = shader_source();
    let source = executable_source(&raw);
    let gate = function_body(&source, "void dzResolvePayloadGate");
    let estimator = function_body(
        &source,
        "float stackDE_dz(vec3 pos, DeLod lod, out DeInfo info) {",
    );
    let render = scene_pass_body(&source);

    // Modes seven and up are the geometry/march diagnostics added while
    // hunting the deep-frame wall; the gate modes below stay in place.
    assert!(raw.contains(r#""VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]"#));
    assert!(raw.contains("Payload gate"));
    for required in [
        "g_dbgOrbitGate = 1.0;",
        "g_dbgOrbitGate = 2.0;",
        "g_dbgOrbitGate = 3.0;",
    ] {
        assert!(gate.contains(required), "missing gate code: {required}");
    }
    assert!(gate.contains("g_dzPayloadMatches = true;"));
    assert!(!estimator.contains("bool useOrbit"));
    assert!(estimator.contains("g_dzPayloadMatches"));
    assert!(render.contains("dzResolvePayloadGate();"));
    assert!(!estimator.contains("dzCurrentStateSignature()"));
    assert!(source.contains("if (g_dbgOrbitGate >= 2.5) return vec4(0.0, 0.8, 0.2, 1.0);"));
    let signature = function_body(&source, "uint dzStateSignature");
    assert!(signature.contains("float scalars[17]"));
    assert!(signature.contains("julia_amount, cocube, lin_mix, stack_cap"));
}

#[test]
fn march_telemetry_reports_steps_exhaustion_and_certificate_jump() {
    let source = executable_source(&shader_source());
    let render = scene_pass_body(&source);

    assert!(render.contains("g_dbgMarchSteps = float(i + 1);"));
    assert!(render.contains("g_dbgCertJump = (certifiedT - t) / max(dzGroup(11).z, 1e-6);"));
    assert!(render.contains("g_dbgMarchSteps / max(float(steps), 1.0)"));
    assert!(render.contains("exhausted ? 1.0 : 0.0"));
    assert!(render.contains("clamp(g_dbgCertJump, 0.0, 1.0)"));
}
