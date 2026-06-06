//! Two-stage face analyzer: BlazeFace detection → Face Landmarks (478 points).
//!
//! Stage 1 (BlazeFace): fast 128×128 face detector producing bboxes + 6 keypoints.
//! Stage 2 (Face Landmarks): 256×256 face mesh producing 478 landmark points per face.
//!
//! Produces:
//! - Scalars: `face_count`, `face_x`, `face_y`, `face_size`, `face_rotation`
//! - Textures: `landmarks` (RGBA8 texture with mesh contours rendered)

use std::collections::HashMap;

use ndarray::Array4;

use super::traits::{
    Analyzer, AnalyzerInput, AnalyzerSchema, AnalyzerSnapshot, ScalarOutputDef, TextureData,
    TextureOutputDef,
};

/// BlazeFace model input resolution.
const MODEL_SIZE: u32 = 128;
/// Face landmarks mesh model input resolution.
const MESH_MODEL_SIZE: u32 = 256;
/// Number of mesh landmarks produced by the face landmarks model.
const NUM_MESH_LANDMARKS: usize = 478;
/// Default confidence threshold for face detection.
const DEFAULT_CONFIDENCE: f32 = 0.5;
/// Default IOU threshold for non-maximum suppression.
const DEFAULT_IOU_THRESHOLD: f32 = 0.3;
/// Maximum detections the model will return.
const DEFAULT_MAX_DETECTIONS: i64 = 10;
/// Number of BlazeFace facial landmarks per detection.
const NUM_BLAZE_LANDMARKS: usize = 6;
/// Resolution of the rendered wireframe overlay texture.
const OVERLAY_SIZE: u32 = 512;
/// Maximum number of faces encoded in data textures.
const MAX_FACES: usize = 10;
/// Width of the face data texture (pixels per row).
const FACE_DATA_W: usize = 480;
/// Width of the dossier text texture (pixels per row).
const DOSSIER_TEX_W: usize = 48;
/// End-of-text sentinel value in dossier text texture.
const END_SENTINEL: u8 = 255;
/// Line-break sentinel value in dossier text texture.
const LINE_BREAK_SENTINEL: u8 = 254;

// ── MediaPipe face mesh contour connections ──────────────────────────────────

/// Face oval (36 edges).
const FACE_OVAL: &[(u16, u16)] = &[
    (10, 338),
    (338, 297),
    (297, 332),
    (332, 284),
    (284, 251),
    (251, 389),
    (389, 356),
    (356, 454),
    (454, 323),
    (323, 361),
    (361, 288),
    (288, 397),
    (397, 365),
    (365, 379),
    (379, 378),
    (378, 400),
    (400, 377),
    (377, 152),
    (152, 148),
    (148, 176),
    (176, 149),
    (149, 150),
    (150, 136),
    (136, 172),
    (172, 58),
    (58, 132),
    (132, 93),
    (93, 234),
    (234, 127),
    (127, 162),
    (162, 21),
    (21, 54),
    (54, 103),
    (103, 67),
    (67, 109),
    (109, 10),
];

/// Lips outer (20 edges).
const LIPS_OUTER: &[(u16, u16)] = &[
    (61, 146),
    (146, 91),
    (91, 181),
    (181, 84),
    (84, 17),
    (17, 314),
    (314, 405),
    (405, 321),
    (321, 375),
    (375, 291),
    (61, 185),
    (185, 40),
    (40, 39),
    (39, 37),
    (37, 0),
    (0, 267),
    (267, 269),
    (269, 270),
    (270, 409),
    (409, 291),
];

/// Lips inner (20 edges).
const LIPS_INNER: &[(u16, u16)] = &[
    (78, 95),
    (95, 88),
    (88, 178),
    (178, 87),
    (87, 14),
    (14, 317),
    (317, 402),
    (402, 318),
    (318, 324),
    (324, 308),
    (78, 191),
    (191, 80),
    (80, 81),
    (81, 82),
    (82, 13),
    (13, 312),
    (312, 311),
    (311, 310),
    (310, 415),
    (415, 308),
];

/// Left eye (16 edges).
const LEFT_EYE: &[(u16, u16)] = &[
    (263, 249),
    (249, 390),
    (390, 373),
    (373, 374),
    (374, 380),
    (380, 381),
    (381, 382),
    (382, 362),
    (263, 466),
    (466, 388),
    (388, 387),
    (387, 386),
    (386, 385),
    (385, 384),
    (384, 398),
    (398, 362),
];

/// Right eye (16 edges).
const RIGHT_EYE: &[(u16, u16)] = &[
    (33, 7),
    (7, 163),
    (163, 144),
    (144, 145),
    (145, 153),
    (153, 154),
    (154, 155),
    (155, 133),
    (33, 246),
    (246, 161),
    (161, 160),
    (160, 159),
    (159, 158),
    (158, 157),
    (157, 173),
    (173, 133),
];

/// Left eyebrow (8 edges).
const LEFT_EYEBROW: &[(u16, u16)] = &[
    (276, 283),
    (283, 282),
    (282, 295),
    (295, 285),
    (300, 293),
    (293, 334),
    (334, 296),
    (296, 336),
];

/// Right eyebrow (8 edges).
const RIGHT_EYEBROW: &[(u16, u16)] = &[
    (46, 53),
    (53, 52),
    (52, 65),
    (65, 55),
    (70, 63),
    (63, 105),
    (105, 66),
    (66, 107),
];

/// Left iris (4 edges).
const LEFT_IRIS: &[(u16, u16)] = &[(474, 475), (475, 476), (476, 477), (477, 474)];

/// Right iris (4 edges).
const RIGHT_IRIS: &[(u16, u16)] = &[(469, 470), (470, 471), (471, 472), (472, 469)];

/// Nose (25 edges).
const NOSE: &[(u16, u16)] = &[
    (168, 6),
    (6, 197),
    (197, 195),
    (195, 5),
    (5, 4),
    (4, 1),
    (1, 19),
    (19, 94),
    (94, 2),
    (98, 97),
    (97, 2),
    (2, 326),
    (326, 327),
    (327, 294),
    (294, 278),
    (278, 344),
    (344, 440),
    (440, 275),
    (275, 4),
    (4, 45),
    (45, 220),
    (220, 115),
    (115, 48),
    (48, 64),
    (64, 98),
];

/// Dossier profile options — assigned randomly per face.
const PROFILES: &[&str] = &[
    "WOOK",
    "PROTO-WOOK",
    "WOOK ADJACENT",
    "ELDER WOOK",
    "CORPORATE WOOK",
    "LOT KID",
    "RAIL RIDER",
    "TOTEM CARRIER",
    "HAMMOCK SQUATTER",
    "PORT-A-POTTY SCOUT",
    "BASSHEAD",
    "HEADY BRO",
    "SPUNION",
    "FLOW ARTIST",
    "HOOP GIRL",
    "GUY WITH A DIDGERIDOO",
    "UNDERCOVER COP",
    "BRAND AMBASSADOR",
    "INSTAGRAM DJ",
    "SOMEBODYS DAD",
    "LOST UBER DRIVER",
    "MAIN CHARACTER",
    "NPC",
    "SIDE QUEST GIVER",
    "FINAL BOSS",
    "TUTORIAL CHARACTER",
    "VOLUNTEER DESERTER",
    "CRYSTAL VENDOR",
    "NITROUS MAFIA",
    "SOUND CAMP INTERN",
];

/// Status options.
const STATUSES: &[&str] = &[
    "IN A K HOLE",
    "PEAKING",
    "COMING UP",
    "EGO DEATH IN PROGRESS",
    "VIBING",
    "TRANSCENDING",
    "BUFFERING",
    "SEARCHING FOR CAMP",
    "LOST BOTH SHOES",
    "FOLLOWING WRONG TOTEM",
    "NEEDS WATER DESPERATELY",
    "TRADING PINS",
    "SELLING GRILLED CHEESE",
    "BUILDING A CRYSTAL GRID",
    "CHARGING PHONE AT MEDICAL",
    "HAS NOT SLEPT SINCE THURSDAY",
    "HOUR 3 OF A DRUM CIRCLE",
    "WILL ASK FOR A CIGARETTE",
    "ABOUT TO PROPOSE TO STRANGER",
    "BECAME ONE WITH THE BASS",
    "COMMUNICATING TELEPATHICALLY",
    "DOWNLOADING COSMIC DATA",
    "REBOOTING CHAKRAS",
    "ASTRAL PROJECTING",
    "LOST IN THE SAUCE",
    "ACHIEVING NIRVANA",
    "FORGETTING NAME",
    "FINDING THIRD EYE",
];

/// Confidence level descriptions.
const CONFIDENCES: &[&str] = &[
    "VIBES CONFIRMED",
    "JUST LOOK MAN",
    "BEYOND CERTAINTY",
    "THE THIRD EYE KNOWS",
    "TRUST THE PROCESS",
    "ITS GIVING",
    "NO CAP",
    "SOURCES: TRUST ME BRO",
    "PROBABLY",
    "ABSOLUTELY",
    "WITHOUT QUESTION",
    "SPIRITUALLY VERIFIED",
];

/// Pre-assigned dossier for a tracked face. Indices into the content arrays.
#[derive(Debug, Clone)]
struct Dossier {
    profile_idx: usize,
    status_idx: usize,
    confidence_idx: usize,
    threat_score: u8,  // 1-10
    social_credit: u8, // 12-45
    credit_score: u16, // 300-450
}

impl Dossier {
    /// Generate a deterministic dossier from a seed value.
    fn from_seed(seed: u64) -> Self {
        // Simple LCG-style hash to spread the seed
        let h = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let h2 = h
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let h3 = h2
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        Self {
            profile_idx: (h as usize) % PROFILES.len(),
            status_idx: (h >> 16) as usize % STATUSES.len(),
            confidence_idx: (h >> 32) as usize % CONFIDENCES.len(),
            threat_score: ((h2 as u8) % 10) + 1,
            social_credit: ((h2 >> 8) as u8 % 34) + 12,
            credit_score: ((h3 as u16) % 151) + 300,
        }
    }
}

/// Map a character to its MSDF atlas index. Atlas is sorted by Unicode codepoint.
fn char_to_atlas_index(ch: char) -> u8 {
    match ch {
        ' ' => 0,
        '!' => 1,
        '\'' => 2,
        ',' => 3,
        '-' => 4,
        '.' => 5,
        '/' => 6,
        '0'..='9' => 7 + (ch as u8 - b'0'),
        ':' => 17,
        '?' => 18,
        'A'..='Z' => 19 + (ch as u8 - b'A'),
        'a'..='z' => 19 + (ch as u8 - b'a'), // lowercase → uppercase
        _ => 0,                              // space for unsupported
    }
}

/// Detected face with bounding box and landmarks.
#[derive(Debug, Clone)]
struct FaceDetection {
    /// Bounding box [x_min, y_min, x_max, y_max] normalized to [0, 1].
    bbox: [f32; 4],
    /// Landmark keypoints as (x, y) pairs, normalized to [0, 1].
    /// Contains 478 mesh landmarks when face mesh succeeds, or 6 BlazeFace landmarks as fallback.
    landmarks: Vec<(f32, f32)>,
}

/// EMA smoothing factor for face detections. Lower = smoother but more lag.
const SMOOTHING_ALPHA: f32 = 0.15;

pub(crate) struct FaceDetectAnalyzer {
    session: Option<ort::session::Session>,
    /// Face landmarks mesh ONNX session (stage 2).
    mesh_session: Option<ort::session::Session>,
    /// Run options with log level set to Error to suppress per-frame shape warnings.
    run_options: Option<ort::session::RunOptions>,
    confidence_threshold: f32,
    iou_threshold: f32,
    max_detections: i64,
    /// Pre-allocated buffer for RGB input (128*128*3 floats, CHW).
    rgb_buffer: Vec<f32>,
    /// Pre-allocated buffer for mesh model input (256*256*3 floats, NHWC).
    mesh_rgb_buffer: Vec<f32>,
    /// Pre-allocated buffer for the landmarks texture output.
    landmark_tex_buffer: Vec<u8>,
    /// Pre-allocated buffer for encoded face data texture.
    face_data_buffer: Vec<u8>,
    /// Pre-allocated buffer for encoded dossier text texture.
    dossier_text_buffer: Vec<u8>,
    /// Previous frame's smoothed detections for EMA filtering.
    prev_detections: Vec<FaceDetection>,
    /// Assigned dossiers for tracked faces, keyed by a stable hash of face position.
    dossiers: Vec<Dossier>,
}

impl FaceDetectAnalyzer {
    pub(crate) fn new() -> Self {
        Self {
            session: None,
            mesh_session: None,
            run_options: None,
            confidence_threshold: DEFAULT_CONFIDENCE,
            iou_threshold: DEFAULT_IOU_THRESHOLD,
            max_detections: DEFAULT_MAX_DETECTIONS,
            rgb_buffer: vec![0.0f32; (MODEL_SIZE * MODEL_SIZE * 3) as usize],
            mesh_rgb_buffer: vec![0.0f32; (MESH_MODEL_SIZE * MESH_MODEL_SIZE * 3) as usize],
            landmark_tex_buffer: vec![0u8; (OVERLAY_SIZE * OVERLAY_SIZE * 4) as usize],
            face_data_buffer: vec![0u8; FACE_DATA_W * MAX_FACES * 4],
            dossier_text_buffer: vec![0u8; DOSSIER_TEX_W * MAX_FACES * 4],
            prev_detections: Vec::new(),
            dossiers: Vec::new(),
        }
    }

    /// Smooth detections using exponential moving average against previous frame.
    fn smooth_detections(&mut self, raw: Vec<FaceDetection>) -> Vec<FaceDetection> {
        if self.prev_detections.is_empty() {
            self.prev_detections = raw.clone();
            return raw;
        }

        let alpha = SMOOTHING_ALPHA;
        let one_minus = 1.0 - alpha;

        // Match detections by index (simple pairing — works well for stable single-face)
        let mut smoothed = Vec::with_capacity(raw.len());
        for (i, det) in raw.iter().enumerate() {
            if let Some(prev) = self.prev_detections.get(i) {
                let bbox = [
                    alpha * det.bbox[0] + one_minus * prev.bbox[0],
                    alpha * det.bbox[1] + one_minus * prev.bbox[1],
                    alpha * det.bbox[2] + one_minus * prev.bbox[2],
                    alpha * det.bbox[3] + one_minus * prev.bbox[3],
                ];
                let count = det.landmarks.len().min(prev.landmarks.len());
                let mut landmarks = Vec::with_capacity(det.landmarks.len());
                for k in 0..count {
                    landmarks.push((
                        alpha * det.landmarks[k].0 + one_minus * prev.landmarks[k].0,
                        alpha * det.landmarks[k].1 + one_minus * prev.landmarks[k].1,
                    ));
                }
                // If current has more landmarks than prev, append unsmoothed
                for k in count..det.landmarks.len() {
                    landmarks.push(det.landmarks[k]);
                }
                smoothed.push(FaceDetection { bbox, landmarks });
            } else {
                smoothed.push(det.clone());
            }
        }

        self.prev_detections = smoothed.clone();
        smoothed
    }

    /// Downsample and convert RGBA frame to CHW RGB float32 normalized [0, 1].
    fn preprocess(&mut self, input: &AnalyzerInput) -> Array4<f32> {
        let src_w = input.width as usize;
        let src_h = input.height as usize;
        let dst = MODEL_SIZE as usize;

        for dy in 0..dst {
            for dx in 0..dst {
                // Nearest-neighbor downsample (fast, sufficient at 128x128)
                let sx = ((dx as f32 * src_w as f32) / dst as f32) as usize;
                let sy = ((dy as f32 * src_h as f32) / dst as f32) as usize;
                let sx = sx.min(src_w - 1);
                let sy = sy.min(src_h - 1);
                let src_idx = (sy * src_w + sx) * 4;

                let r = input.frame[src_idx] as f32 / 255.0;
                let g = input.frame[src_idx + 1] as f32 / 255.0;
                let b = input.frame[src_idx + 2] as f32 / 255.0;

                // CHW layout: channel * H * W + y * W + x
                let pixel = dy * dst + dx;
                self.rgb_buffer[pixel] = r;
                self.rgb_buffer[dst * dst + pixel] = g;
                self.rgb_buffer[2 * dst * dst + pixel] = b;
            }
        }

        Array4::from_shape_vec((1, 3, dst, dst), self.rgb_buffer.clone())
            .expect("shape mismatch in preprocess")
    }

    /// Parse model output into face detections from raw shape + data.
    ///
    /// BlazeFace output layout per detection (16 floats):
    ///   [0..4] = bounding box as **[ymin, xmin, ymax, xmax]** (normalized 0–1)
    ///   [4..16] = 6 landmark keypoints as (x, y) pairs (normalized 0–1)
    ///
    /// Handles both 3D output `[1, N, 16]` and 2D output `[1, 16]` (ONNX Runtime
    /// squeezes the middle dimension when N=1).
    fn postprocess_raw(&self, shape: &[i64], data: &[f32]) -> Vec<FaceDetection> {
        let (n_faces, cols) = if shape.len() == 3 {
            if shape[1] == 0 {
                return Vec::new();
            }
            (shape[1] as usize, shape[2] as usize)
        } else if shape.len() == 2 && shape[1] == 16 {
            (1_usize, 16_usize)
        } else {
            return Vec::new();
        };
        let mut detections = Vec::with_capacity(n_faces);
        for i in 0..n_faces {
            let offset = i * cols;
            if offset + 16 > data.len() {
                break;
            }
            let row = &data[offset..offset + 16];
            // BlazeFace bbox order: [ymin, xmin, ymax, xmax] → we store [xmin, ymin, xmax, ymax]
            let bbox = [
                row[1].clamp(0.0, 1.0), // xmin
                row[0].clamp(0.0, 1.0), // ymin
                row[3].clamp(0.0, 1.0), // xmax
                row[2].clamp(0.0, 1.0), // ymax
            ];
            let mut landmarks = Vec::with_capacity(NUM_BLAZE_LANDMARKS);
            for k in 0..NUM_BLAZE_LANDMARKS {
                landmarks.push((
                    row[4 + k * 2].clamp(0.0, 1.0),
                    row[4 + k * 2 + 1].clamp(0.0, 1.0),
                ));
            }
            detections.push(FaceDetection { bbox, landmarks });
        }
        detections
    }

    /// Render landmark wireframe contours into an RGBA texture for shader consumption.
    fn render_landmarks_texture(&mut self, detections: &[FaceDetection]) -> TextureData {
        let size = OVERLAY_SIZE as f32;
        self.landmark_tex_buffer.fill(0);

        let white: [u8; 4] = [255, 255, 255, 220];
        let line_color: [u8; 4] = [255, 255, 255, 140];

        for det in detections.iter() {
            // Convert landmarks to pixel coords
            let pts: Vec<(i32, i32)> = det
                .landmarks
                .iter()
                .map(|(lx, ly)| ((lx * size) as i32, (ly * size) as i32))
                .collect();

            if pts.len() >= NUM_MESH_LANDMARKS {
                // Full 478-point mesh: draw contour connections
                let all_contours: &[&[(u16, u16)]] = &[
                    FACE_OVAL,
                    LIPS_OUTER,
                    LIPS_INNER,
                    LEFT_EYE,
                    RIGHT_EYE,
                    LEFT_EYEBROW,
                    RIGHT_EYEBROW,
                    LEFT_IRIS,
                    RIGHT_IRIS,
                    NOSE,
                ];
                for contour in all_contours {
                    for &(a, b) in *contour {
                        let (a, b) = (a as usize, b as usize);
                        if a < pts.len() && b < pts.len() {
                            self.draw_line(pts[a].0, pts[a].1, pts[b].0, pts[b].1, line_color);
                        }
                    }
                }

                // Dots at key feature points: eyes, nose tip, mouth corners, iris centers
                let key_points: &[usize] = &[
                    33, 263, // eye centers
                    1,   // nose tip
                    61, 291, // mouth corners
                    13, 14, // lip centers
                    468, 473, // iris centers
                ];
                for &idx in key_points {
                    if idx < pts.len() {
                        self.draw_dot(pts[idx].0, pts[idx].1, 3, white);
                    }
                }
            } else {
                // Fallback: 6-point BlazeFace wireframe
                let connections: &[(usize, usize)] = &[
                    (0, 1),
                    (0, 2),
                    (1, 2),
                    (2, 3),
                    (0, 4),
                    (1, 5),
                    (4, 3),
                    (5, 3),
                ];
                for &(a, b) in connections {
                    if a < pts.len() && b < pts.len() {
                        self.draw_line(pts[a].0, pts[a].1, pts[b].0, pts[b].1, line_color);
                    }
                }
                for &(px, py) in &pts {
                    self.draw_dot(px, py, 5, white);
                }
            }
        }

        TextureData {
            width: OVERLAY_SIZE,
            height: OVERLAY_SIZE,
            format: "rgba8unorm".into(),
            data: self.landmark_tex_buffer.clone(),
        }
    }

    /// Encode face bounding boxes and dossier scores into a FACE_DATA_W × MAX_FACES RGBA8 texture.
    fn encode_face_data_texture(
        &mut self,
        detections: &[FaceDetection],
        dossiers: &[Dossier],
    ) -> TextureData {
        self.face_data_buffer.fill(0);

        let face_count = detections.len().min(MAX_FACES);

        for (f, det) in detections.iter().enumerate().take(MAX_FACES) {
            let row_offset = f * FACE_DATA_W * 4;

            // Pixel 0: bbox
            let px0 = row_offset;
            self.face_data_buffer[px0] = (det.bbox[0].clamp(0.0, 1.0) * 255.0) as u8;
            self.face_data_buffer[px0 + 1] = (det.bbox[1].clamp(0.0, 1.0) * 255.0) as u8;
            self.face_data_buffer[px0 + 2] = (det.bbox[2].clamp(0.0, 1.0) * 255.0) as u8;
            self.face_data_buffer[px0 + 3] = (det.bbox[3].clamp(0.0, 1.0) * 255.0) as u8;

            // Pixel 1: face_count in R channel
            let px1 = row_offset + 4;
            self.face_data_buffer[px1] = face_count as u8;

            // Pixel 2: dossier scores
            if let Some(dossier) = dossiers.get(f) {
                let px2 = row_offset + 8;
                self.face_data_buffer[px2] = dossier.threat_score.min(10) * 25;
                self.face_data_buffer[px2 + 1] = dossier.social_credit.min(50) * 5;
                self.face_data_buffer[px2 + 2] = (dossier.credit_score >> 8) as u8;
                self.face_data_buffer[px2 + 3] = (dossier.credit_score & 0xFF) as u8;
            }
        }

        TextureData {
            width: FACE_DATA_W as u32,
            height: MAX_FACES as u32,
            format: "rgba8unorm".into(),
            data: self.face_data_buffer.clone(),
        }
    }

    /// Encode dossier text as atlas indices into a DOSSIER_TEX_W × MAX_FACES RGBA8 texture.
    fn encode_dossier_text_texture(&mut self, dossiers: &[Dossier]) -> TextureData {
        self.dossier_text_buffer.fill(END_SENTINEL);

        for (f, dossier) in dossiers.iter().enumerate().take(MAX_FACES) {
            let profile = PROFILES[dossier.profile_idx % PROFILES.len()];
            let status = STATUSES[dossier.status_idx % STATUSES.len()];
            let confidence = CONFIDENCES[dossier.confidence_idx % CONFIDENCES.len()];

            let lines: Vec<String> = vec![
                "-- SUBJECT DOSSIER --".to_string(),
                format!("PROFILE: {}", profile),
                format!("STATUS: {}", status),
                format!("CONFIDENCE: {}", confidence),
                format!("THREAT: {}/10", dossier.threat_score),
                format!("SOCIAL CREDIT: {}/100", dossier.social_credit),
                format!("CREDIT SCORE: {}", dossier.credit_score),
            ];

            let row_offset = f * DOSSIER_TEX_W * 4;
            let mut byte_idx = 0;
            let max_bytes = DOSSIER_TEX_W * 4;

            for (line_idx, line) in lines.iter().enumerate() {
                for ch in line.chars() {
                    if byte_idx >= max_bytes {
                        break;
                    }
                    self.dossier_text_buffer[row_offset + byte_idx] = char_to_atlas_index(ch);
                    byte_idx += 1;
                }
                // Add line break sentinel (except after last line)
                if line_idx < lines.len() - 1 && byte_idx < max_bytes {
                    self.dossier_text_buffer[row_offset + byte_idx] = LINE_BREAK_SENTINEL;
                    byte_idx += 1;
                }
            }
            // Remaining bytes stay as END_SENTINEL (255) from the fill
        }

        TextureData {
            width: DOSSIER_TEX_W as u32,
            height: MAX_FACES as u32,
            format: "rgba8unorm".into(),
            data: self.dossier_text_buffer.clone(),
        }
    }

    /// Draw a 1px anti-aliased line between two points (Bresenham's algorithm).
    fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: [u8; 4]) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;
        loop {
            self.set_pixel_i32(cx, cy, color);
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    fn draw_dot(&mut self, cx: i32, cy: i32, radius: i32, color: [u8; 4]) {
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy <= radius * radius {
                    self.set_pixel_i32(cx + dx, cy + dy, color);
                }
            }
        }
    }

    fn set_pixel_i32(&mut self, x: i32, y: i32, color: [u8; 4]) {
        let size = OVERLAY_SIZE as i32;
        if x >= 0 && x < size && y >= 0 && y < size {
            let idx = (y as usize * OVERLAY_SIZE as usize + x as usize) * 4;
            self.landmark_tex_buffer[idx..idx + 4].copy_from_slice(&color);
        }
    }
}

impl Analyzer for FaceDetectAnalyzer {
    fn analyzer_type(&self) -> &str {
        "face_detect"
    }

    fn output_schema(&self) -> AnalyzerSchema {
        AnalyzerSchema {
            scalars: vec![
                ScalarOutputDef {
                    name: "face_count".into(),
                    description: "Number of detected faces (normalized: 1 face = 0.1)".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.05,
                },
                ScalarOutputDef {
                    name: "face_x".into(),
                    description: "Primary face center X position".into(),
                    range: (0.0, 1.0),
                    default: 0.5,
                    default_smoothing: 0.15,
                },
                ScalarOutputDef {
                    name: "face_y".into(),
                    description: "Primary face center Y position".into(),
                    range: (0.0, 1.0),
                    default: 0.5,
                    default_smoothing: 0.15,
                },
                ScalarOutputDef {
                    name: "face_size".into(),
                    description: "Primary face bounding box area (normalized)".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.15,
                },
                ScalarOutputDef {
                    name: "face_rotation".into(),
                    description: "Primary face rotation from eye angle".into(),
                    range: (0.0, 1.0),
                    default: 0.5,
                    default_smoothing: 0.2,
                },
            ],
            textures: vec![
                TextureOutputDef {
                    name: "landmarks".into(),
                    description: "Face wireframe contours on RGBA texture".into(),
                    format: "rgba8unorm".into(),
                },
                TextureOutputDef {
                    name: "face_data".into(),
                    description: "Face bounding boxes and scores encoded as RGBA data".into(),
                    format: "rgba8unorm".into(),
                },
                TextureOutputDef {
                    name: "dossier_text".into(),
                    description: "Pre-encoded dossier text character indices".into(),
                    format: "rgba8unorm".into(),
                },
            ],
        }
    }

    fn init(&mut self, options: &serde_json::Value) -> anyhow::Result<()> {
        if let Some(conf) = options.get("confidence").and_then(|v| v.as_f64()) {
            self.confidence_threshold = conf as f32;
        }
        if let Some(iou) = options.get("iou_threshold").and_then(|v| v.as_f64()) {
            self.iou_threshold = iou as f32;
        }
        if let Some(max) = options.get("max_faces").and_then(|v| v.as_i64()) {
            self.max_detections = max;
        }

        let model_path = options
            .get("model_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .unwrap_or_else(Self::find_model_path);

        log::info!(
            "FaceDetectAnalyzer: loading model from '{model_path}' \
             (conf={}, iou={}, max={})",
            self.confidence_threshold,
            self.iou_threshold,
            self.max_detections
        );

        let session = ort::session::Session::builder()
            .map_err(|e| anyhow::anyhow!("ort session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| anyhow::anyhow!("ort intra_threads: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| anyhow::anyhow!("ort load model '{model_path}': {e}"))?;
        self.session = Some(session);

        // Suppress per-frame ORT shape mismatch warnings from the NMS output layer.
        // The model declares {1,896,16} but NMS dynamically produces {1,N,16}.
        let mut run_opts =
            ort::session::RunOptions::new().map_err(|e| anyhow::anyhow!("ort run options: {e}"))?;
        run_opts
            .set_log_level(ort::logging::LogLevel::Error)
            .map_err(|e| anyhow::anyhow!("ort set log level: {e}"))?;
        self.run_options = Some(run_opts);

        log::info!("FaceDetectAnalyzer: BlazeFace session created successfully");

        // Stage 2: load face landmarks mesh model
        let mesh_model_path = options
            .get("mesh_model_path")
            .and_then(|v| v.as_str())
            .map(|s| s.to_owned())
            .unwrap_or_else(Self::find_mesh_model_path);

        match (|| -> Result<ort::session::Session, anyhow::Error> {
            let s = ort::session::Session::builder()
                .map_err(|e| anyhow::anyhow!("mesh builder: {e}"))?
                .with_intra_threads(1)
                .map_err(|e| anyhow::anyhow!("mesh intra_threads: {e}"))?
                .commit_from_file(&mesh_model_path)
                .map_err(|e| anyhow::anyhow!("mesh load: {e}"))?;
            Ok(s)
        })() {
            Ok(mesh_session) => {
                let input_names: Vec<_> = mesh_session.inputs().iter().map(|i| i.name()).collect();
                let output_names: Vec<_> =
                    mesh_session.outputs().iter().map(|o| o.name()).collect();
                log::info!(
                    "FaceDetectAnalyzer: mesh model loaded from '{mesh_model_path}' \
                     inputs={input_names:?} outputs={output_names:?}"
                );
                self.mesh_session = Some(mesh_session);
            }
            Err(e) => {
                log::warn!(
                    "FaceDetectAnalyzer: mesh model not loaded from '{mesh_model_path}': {e}. \
                     Falling back to 6-point BlazeFace landmarks only."
                );
            }
        }

        Ok(())
    }

    fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot> {
        // Extract config values and preprocess before borrowing session
        let input_tensor = self.preprocess(input);
        let conf_threshold = self.confidence_threshold;
        let max_detections = self.max_detections;
        let iou_threshold = self.iou_threshold;

        let conf_arr = ndarray::Array1::from_vec(vec![conf_threshold]);
        let max_arr = ndarray::Array1::from_vec(vec![max_detections]);
        let iou_arr = ndarray::Array1::from_vec(vec![iou_threshold]);

        let image_ref = ort::value::TensorRef::from_array_view(input_tensor.view())
            .map_err(|e| anyhow::anyhow!("ort image tensor: {e}"))?;
        let conf_ref = ort::value::TensorRef::from_array_view(conf_arr.view())
            .map_err(|e| anyhow::anyhow!("ort conf tensor: {e}"))?;
        let max_ref = ort::value::TensorRef::from_array_view(max_arr.view())
            .map_err(|e| anyhow::anyhow!("ort max tensor: {e}"))?;
        let iou_ref = ort::value::TensorRef::from_array_view(iou_arr.view())
            .map_err(|e| anyhow::anyhow!("ort iou tensor: {e}"))?;

        let inputs = ort::inputs![
            "image" => image_ref,
            "conf_threshold" => conf_ref,
            "max_detections" => max_ref,
            "iou_threshold" => iou_ref,
        ];

        // Borrow session + run_options, run inference, extract results, then drop
        let session = self
            .session
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("ONNX session not initialized"))?;
        let run_options = self
            .run_options
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("RunOptions not initialized"))?;

        let outputs = session
            .run_with_options(inputs, run_options)
            .map_err(|e| anyhow::anyhow!("ort inference: {e}"))?;

        let boxes_value = &outputs["selectedBoxes"];
        let (shape, data) = boxes_value
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow::anyhow!("ort extract: {e}"))?;

        // Clone detection data so we can drop the outputs borrow
        let shape_vec: Vec<i64> = shape.iter().copied().collect();
        let data_vec: Vec<f32> = data.to_vec();
        drop(outputs);

        let mut raw_detections = self.postprocess_raw(&shape_vec, &data_vec);

        // Stage 2: run face mesh on each detected face to get 478 landmarks.
        // If mesh fails (e.g. extreme angle), the detection keeps its 6 BlazeFace landmarks.
        if self.mesh_session.is_some() {
            for det in &mut raw_detections {
                if let Some(mesh_landmarks) = self.run_face_mesh(input, &det.bbox) {
                    det.landmarks = mesh_landmarks;
                }
            }
        }

        let detections = self.smooth_detections(raw_detections);

        // Assign stable dossiers: grow or shrink the dossier vec to match detection count.
        // Each face gets a deterministic dossier seeded from its bbox center position.
        while self.dossiers.len() < detections.len() {
            let det = &detections[self.dossiers.len()];
            let cx = ((det.bbox[0] + det.bbox[2]) * 5000.0) as u64;
            let cy = ((det.bbox[1] + det.bbox[3]) * 5000.0) as u64;
            let seed = cx.wrapping_mul(73856093) ^ cy.wrapping_mul(19349663);
            self.dossiers.push(Dossier::from_seed(seed));
        }
        self.dossiers.truncate(detections.len());

        let mut scalars = HashMap::with_capacity(5);
        let face_count = detections.len();
        scalars.insert("face_count".into(), (face_count as f32 * 0.1).min(1.0));

        if let Some(primary) = detections.iter().max_by(|a, b| {
            let area_a = (a.bbox[2] - a.bbox[0]) * (a.bbox[3] - a.bbox[1]);
            let area_b = (b.bbox[2] - b.bbox[0]) * (b.bbox[3] - b.bbox[1]);
            area_a
                .partial_cmp(&area_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        }) {
            let cx = (primary.bbox[0] + primary.bbox[2]) / 2.0;
            let cy = (primary.bbox[1] + primary.bbox[3]) / 2.0;
            let w = primary.bbox[2] - primary.bbox[0];
            let h = primary.bbox[3] - primary.bbox[1];
            let area = (w * h).sqrt();

            // Use mesh landmark indices for eye positions if we have 478 points,
            // otherwise fall back to BlazeFace 6-point indices.
            let (le, re) = if primary.landmarks.len() >= NUM_MESH_LANDMARKS {
                // Mesh: index 33 = right eye center, index 263 = left eye center
                (primary.landmarks[263], primary.landmarks[33])
            } else if primary.landmarks.len() >= 2 {
                (primary.landmarks[0], primary.landmarks[1])
            } else {
                ((0.5, 0.5), (0.5, 0.5))
            };
            let eye_angle = (re.1 - le.1).atan2(re.0 - le.0);
            let rotation = (eye_angle / std::f32::consts::FRAC_PI_2 + 1.0) / 2.0;

            scalars.insert("face_x".into(), cx);
            scalars.insert("face_y".into(), cy);
            scalars.insert("face_size".into(), area.clamp(0.0, 1.0));
            scalars.insert("face_rotation".into(), rotation.clamp(0.0, 1.0));
        } else {
            scalars.insert("face_x".into(), 0.5);
            scalars.insert("face_y".into(), 0.5);
            scalars.insert("face_size".into(), 0.0);
            scalars.insert("face_rotation".into(), 0.5);
        }

        let dossiers = self.dossiers.clone();
        let landmarks_tex = self.render_landmarks_texture(&detections);
        let face_data_tex = self.encode_face_data_texture(&detections, &dossiers);
        let dossier_text_tex = self.encode_dossier_text_texture(&dossiers);

        let mut textures = HashMap::with_capacity(3);
        textures.insert("landmarks".into(), landmarks_tex);
        textures.insert("face_data".into(), face_data_tex);
        textures.insert("dossier_text".into(), dossier_text_tex);

        Ok(AnalyzerSnapshot {
            scalars,
            textures,
            timestamp: input.timestamp,
        })
    }

    fn shutdown(&mut self) {
        self.session = None;
        self.mesh_session = None;
        log::info!("FaceDetectAnalyzer: sessions released");
    }
}

impl FaceDetectAnalyzer {
    /// Search for the BlazeFace model file in standard locations.
    fn find_model_path() -> String {
        if let Ok(exe_path) = std::env::current_exe() {
            // macOS .app bundle: Contents/Resources/models/blaze.onnx
            if let Some(bundle_dir) = exe_path.parent().and_then(|p| p.parent()) {
                let p = bundle_dir.join("Resources/models/blaze.onnx");
                if p.exists() {
                    return p.to_string_lossy().into_owned();
                }
            }
            if let Some(exe_dir) = exe_path.parent() {
                let p = exe_dir.join("models/blaze.onnx");
                if p.exists() {
                    return p.to_string_lossy().into_owned();
                }
            }
        }
        "models/blaze.onnx".into()
    }

    /// Search for the face landmarks mesh model file in standard locations.
    fn find_mesh_model_path() -> String {
        let filename = "face_landmarks_detector.onnx";
        if let Ok(exe_path) = std::env::current_exe() {
            // macOS .app bundle: Contents/Resources/models/
            if let Some(bundle_dir) = exe_path.parent().and_then(|p| p.parent()) {
                let p = bundle_dir.join("Resources/models").join(filename);
                if p.exists() {
                    return p.to_string_lossy().into_owned();
                }
            }
            if let Some(exe_dir) = exe_path.parent() {
                let p = exe_dir.join("models").join(filename);
                if p.exists() {
                    return p.to_string_lossy().into_owned();
                }
            }
        }
        format!("models/{filename}")
    }

    /// Crop a square face region from input frame with margin, resize to 256×256 NHWC float32.
    /// Uses max(w, h) so the crop is always square — avoids aspect-ratio distortion that
    /// confuses the mesh model on bearded/tall faces.
    fn preprocess_face_crop(&mut self, input: &AnalyzerInput, bbox: &[f32; 4]) -> Array4<f32> {
        let cx = (bbox[0] + bbox[2]) / 2.0;
        let cy = (bbox[1] + bbox[3]) / 2.0;
        let w = bbox[2] - bbox[0];
        let h = bbox[3] - bbox[1];
        let side = w.max(h);
        let margin = 0.4;
        let half = side * (0.5 + margin);
        let crop_x0 = (cx - half).max(0.0);
        let crop_y0 = (cy - half).max(0.0);
        let crop_x1 = (cx + half).min(1.0);
        let crop_y1 = (cy + half).min(1.0);

        let dst = MESH_MODEL_SIZE as usize;
        let src_w = input.width as usize;
        let src_h = input.height as usize;

        for dy in 0..dst {
            for dx in 0..dst {
                let fx = crop_x0 + (dx as f32 / dst as f32) * (crop_x1 - crop_x0);
                let fy = crop_y0 + (dy as f32 / dst as f32) * (crop_y1 - crop_y0);
                let sx = ((fx * src_w as f32) as usize).min(src_w - 1);
                let sy = ((fy * src_h as f32) as usize).min(src_h - 1);
                let src_idx = (sy * src_w + sx) * 4;

                // NHWC layout: row * W * 3 + col * 3 + channel
                let dst_idx = (dy * dst + dx) * 3;
                self.mesh_rgb_buffer[dst_idx] = input.frame[src_idx] as f32 / 255.0;
                self.mesh_rgb_buffer[dst_idx + 1] = input.frame[src_idx + 1] as f32 / 255.0;
                self.mesh_rgb_buffer[dst_idx + 2] = input.frame[src_idx + 2] as f32 / 255.0;
            }
        }

        Array4::from_shape_vec((1, dst, dst, 3), self.mesh_rgb_buffer.clone())
            .expect("shape mismatch in face crop")
    }

    /// Run face mesh inference and extract 478 landmarks mapped to global normalized coords.
    fn run_face_mesh(&mut self, input: &AnalyzerInput, bbox: &[f32; 4]) -> Option<Vec<(f32, f32)>> {
        let cx = (bbox[0] + bbox[2]) / 2.0;
        let cy = (bbox[1] + bbox[3]) / 2.0;
        let w = bbox[2] - bbox[0];
        let h = bbox[3] - bbox[1];
        let side = w.max(h);
        let margin = 0.4;
        let half = side * (0.5 + margin);
        let crop_x0 = (cx - half).max(0.0);
        let crop_y0 = (cy - half).max(0.0);
        let crop_x1 = (cx + half).min(1.0);
        let crop_y1 = (cy + half).min(1.0);
        let crop_w = crop_x1 - crop_x0;
        let crop_h = crop_y1 - crop_y0;

        let input_tensor = self.preprocess_face_crop(input, bbox);

        let session = self.mesh_session.as_mut()?;
        let run_options = self.run_options.as_ref()?;

        let tensor_ref = ort::value::TensorRef::from_array_view(input_tensor.view()).ok()?;

        // Use the first input name from the model
        let input_name = session.inputs().first()?.name().to_string();
        let inputs = ort::inputs![input_name.as_str() => tensor_ref];

        let outputs = session.run_with_options(inputs, run_options).ok()?;

        // First output = landmarks tensor, flatten to f32 slice
        let landmarks_value = &outputs[0];
        let (_, landmarks_raw) = landmarks_value.try_extract_tensor::<f32>().ok()?;
        let landmarks_data: Vec<f32> = landmarks_raw.to_vec();

        // Score check if available (second output), apply sigmoid.
        // Use a very low threshold (0.1) so side-facing / tilted faces still get mesh landmarks.
        if outputs.len() > 1 {
            let score_value = &outputs[1];
            if let Ok((_, score_raw)) = score_value.try_extract_tensor::<f32>() {
                let score_data: Vec<f32> = score_raw.to_vec();
                if !score_data.is_empty() {
                    let raw_score = score_data[0];
                    let score = 1.0 / (1.0 + (-raw_score).exp());
                    if score < 0.1 {
                        return None;
                    }
                }
            }
        }

        let mesh_size = MESH_MODEL_SIZE as f32;
        let mut landmarks = Vec::with_capacity(NUM_MESH_LANDMARKS);
        for i in 0..NUM_MESH_LANDMARKS {
            let idx = i * 3;
            if idx + 1 >= landmarks_data.len() {
                break;
            }
            // Landmark coords are in model pixel space (0-256)
            // Convert to global normalized: pixel / model_size * crop_extent + crop_origin
            let lx = (landmarks_data[idx] / mesh_size) * crop_w + crop_x0;
            let ly = (landmarks_data[idx + 1] / mesh_size) * crop_h + crop_y0;
            landmarks.push((lx.clamp(0.0, 1.0), ly.clamp(0.0, 1.0)));
        }

        if landmarks.len() == NUM_MESH_LANDMARKS {
            Some(landmarks)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn schema_has_correct_outputs() {
        let analyzer = FaceDetectAnalyzer::new();
        let schema = analyzer.output_schema();
        let names: Vec<&str> = schema.scalars.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "face_count",
                "face_x",
                "face_y",
                "face_size",
                "face_rotation"
            ]
        );
        assert_eq!(schema.textures.len(), 3);
        assert_eq!(schema.textures[0].name, "landmarks");
        assert_eq!(schema.textures[1].name, "face_data");
        assert_eq!(schema.textures[2].name, "dossier_text");
    }

    #[test]
    fn preprocess_produces_correct_shape() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let input = AnalyzerInput {
            frame: vec![128u8; 64 * 64 * 4],
            width: 64,
            height: 64,
            timestamp: Instant::now(),
        };
        let tensor = analyzer.preprocess(&input);
        assert_eq!(tensor.shape(), &[1, 3, 128, 128]);
        let expected = 128.0 / 255.0;
        let actual = tensor[[0, 0, 0, 0]];
        assert!(
            (actual - expected).abs() < 0.01,
            "expected ~{expected}, got {actual}"
        );
    }

    #[test]
    fn postprocess_empty_returns_empty() {
        let analyzer = FaceDetectAnalyzer::new();
        let shape = [1i64, 0, 16];
        let data: Vec<f32> = vec![];
        let detections = analyzer.postprocess_raw(&shape, &data);
        assert!(detections.is_empty());
    }

    #[test]
    fn postprocess_extracts_detections() {
        let analyzer = FaceDetectAnalyzer::new();
        let shape = [1i64, 2, 16];
        let mut data = vec![0.0f32; 32]; // 2 faces * 16 values
                                         // Face 1: raw = [ymin=0.1, xmin=0.2, ymax=0.5, xmax=0.6]
                                         //  → bbox = [xmin=0.2, ymin=0.1, xmax=0.6, ymax=0.5]
        data[0] = 0.1;
        data[1] = 0.2;
        data[2] = 0.5;
        data[3] = 0.6;
        // Face 2: raw = [ymin=0.3, xmin=0.3, ymax=0.7, xmax=0.8]
        //  → bbox = [xmin=0.3, ymin=0.3, xmax=0.8, ymax=0.7]
        data[16] = 0.3;
        data[17] = 0.3;
        data[18] = 0.7;
        data[19] = 0.8;
        let detections = analyzer.postprocess_raw(&shape, &data);
        assert_eq!(detections.len(), 2);
        // bbox[0] = xmin = data[1]
        assert!((detections[0].bbox[0] - 0.2).abs() < 1e-5);
        // bbox[2] = xmax = data[19]
        assert!((detections[1].bbox[2] - 0.8).abs() < 1e-5);
    }

    #[test]
    fn postprocess_handles_squeezed_2d_shape() {
        // ONNX Runtime squeezes [1, 1, 16] to [1, 16] when only 1 detection
        let analyzer = FaceDetectAnalyzer::new();
        let shape = [1i64, 16];
        let mut data = vec![0.0f32; 16];
        // raw = [ymin=0.2, xmin=0.3, ymax=0.6, xmax=0.7]
        //  → bbox = [xmin=0.3, ymin=0.2, xmax=0.7, ymax=0.6]
        data[0] = 0.2;
        data[1] = 0.3;
        data[2] = 0.6;
        data[3] = 0.7;
        let detections = analyzer.postprocess_raw(&shape, &data);
        assert_eq!(detections.len(), 1);
        // bbox[0] = xmin = data[1]
        assert!((detections[0].bbox[0] - 0.3).abs() < 1e-5);
        // bbox[2] = xmax = data[3]
        assert!((detections[0].bbox[2] - 0.7).abs() < 1e-5);
    }

    #[test]
    fn landmarks_texture_correct_size_blaze_fallback() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let det = FaceDetection {
            bbox: [0.2, 0.2, 0.8, 0.8],
            landmarks: vec![
                (0.35, 0.4),
                (0.65, 0.4),
                (0.5, 0.55),
                (0.5, 0.7),
                (0.25, 0.45),
                (0.75, 0.45),
            ],
        };
        let tex = analyzer.render_landmarks_texture(&[det]);
        assert_eq!(tex.width, OVERLAY_SIZE);
        assert_eq!(tex.height, OVERLAY_SIZE);
        assert_eq!(tex.data.len(), (OVERLAY_SIZE * OVERLAY_SIZE * 4) as usize);
        assert!(tex.data.iter().any(|&b| b > 0));
    }

    #[test]
    fn landmarks_texture_correct_size_mesh() {
        let mut analyzer = FaceDetectAnalyzer::new();
        // Create 478 fake mesh landmarks spread across the face region
        let landmarks: Vec<(f32, f32)> = (0..NUM_MESH_LANDMARKS)
            .map(|i| {
                let t = i as f32 / NUM_MESH_LANDMARKS as f32;
                (0.3 + t * 0.4, 0.3 + t * 0.4)
            })
            .collect();
        let det = FaceDetection {
            bbox: [0.2, 0.2, 0.8, 0.8],
            landmarks,
        };
        let tex = analyzer.render_landmarks_texture(&[det]);
        assert_eq!(tex.width, OVERLAY_SIZE);
        assert_eq!(tex.height, OVERLAY_SIZE);
        assert_eq!(tex.data.len(), (OVERLAY_SIZE * OVERLAY_SIZE * 4) as usize);
        assert!(tex.data.iter().any(|&b| b > 0));
    }

    #[test]
    fn preprocess_face_crop_correct_shape() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let input = AnalyzerInput {
            frame: vec![128u8; 64 * 64 * 4],
            width: 64,
            height: 64,
            timestamp: Instant::now(),
        };
        let bbox = [0.2, 0.2, 0.8, 0.8];
        let tensor = analyzer.preprocess_face_crop(&input, &bbox);
        // NHWC: [1, 256, 256, 3]
        assert_eq!(tensor.shape(), &[1, 256, 256, 3]);
    }

    #[test]
    fn char_to_atlas_index_mapping() {
        // A-Z → 19-44
        assert_eq!(char_to_atlas_index('A'), 19);
        assert_eq!(char_to_atlas_index('Z'), 44);
        assert_eq!(char_to_atlas_index('M'), 31);
        // lowercase maps to uppercase
        assert_eq!(char_to_atlas_index('a'), 19);
        assert_eq!(char_to_atlas_index('z'), 44);
        // 0-9 → 7-16
        assert_eq!(char_to_atlas_index('0'), 7);
        assert_eq!(char_to_atlas_index('9'), 16);
        assert_eq!(char_to_atlas_index('5'), 12);
        // Punctuation
        assert_eq!(char_to_atlas_index(' '), 0);
        assert_eq!(char_to_atlas_index('!'), 1);
        assert_eq!(char_to_atlas_index('\''), 2);
        assert_eq!(char_to_atlas_index(','), 3);
        assert_eq!(char_to_atlas_index('-'), 4);
        assert_eq!(char_to_atlas_index('.'), 5);
        assert_eq!(char_to_atlas_index('/'), 6);
        assert_eq!(char_to_atlas_index(':'), 17);
        assert_eq!(char_to_atlas_index('?'), 18);
        // Unsupported → space (0)
        assert_eq!(char_to_atlas_index('@'), 0);
        assert_eq!(char_to_atlas_index('#'), 0);
    }

    #[test]
    fn encode_face_data_roundtrip() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let det = FaceDetection {
            bbox: [0.2, 0.3, 0.7, 0.8],
            landmarks: vec![(0.5, 0.5)],
        };
        let dossier = Dossier::from_seed(42);
        let tex = analyzer.encode_face_data_texture(&[det], &[dossier.clone()]);

        assert_eq!(tex.width, FACE_DATA_W as u32);
        assert_eq!(tex.height, MAX_FACES as u32);

        // Decode bbox from pixel 0, row 0
        let x0 = tex.data[0] as f32 / 255.0;
        let y0 = tex.data[1] as f32 / 255.0;
        let x1 = tex.data[2] as f32 / 255.0;
        let y1 = tex.data[3] as f32 / 255.0;

        // Byte precision: ~0.004 tolerance
        assert!((x0 - 0.2).abs() < 0.005, "x0={x0}");
        assert!((y0 - 0.3).abs() < 0.005, "y0={y0}");
        assert!((x1 - 0.7).abs() < 0.005, "x1={x1}");
        assert!((y1 - 0.8).abs() < 0.005, "y1={y1}");

        // Face count in pixel 1, channel R
        assert_eq!(tex.data[4], 1);
    }

    #[test]
    fn encode_dossier_text_smoke() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let dossier = Dossier::from_seed(42);
        let tex = analyzer.encode_dossier_text_texture(&[dossier]);

        assert_eq!(tex.width, DOSSIER_TEX_W as u32);
        assert_eq!(tex.height, MAX_FACES as u32);

        // First line: "-- SUBJECT DOSSIER --"
        // '-' = atlas index 4, '-' = 4, ' ' = 0, 'S' = 37, ...
        assert_eq!(tex.data[0], char_to_atlas_index('-'));
        assert_eq!(tex.data[1], char_to_atlas_index('-'));
        assert_eq!(tex.data[2], char_to_atlas_index(' '));
        assert_eq!(tex.data[3], char_to_atlas_index('S'));

        // Verify line break sentinel exists somewhere
        let row_data = &tex.data[0..DOSSIER_TEX_W * 4];
        assert!(
            row_data.iter().any(|&b| b == LINE_BREAK_SENTINEL),
            "Expected line break sentinel in dossier text"
        );

        // Verify end sentinel exists
        assert!(
            row_data.iter().any(|&b| b == END_SENTINEL),
            "Expected end sentinel in dossier text"
        );
    }

    #[test]
    fn face_data_texture_dimensions() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let tex = analyzer.encode_face_data_texture(&[], &[]);
        assert_eq!(tex.width, FACE_DATA_W as u32);
        assert_eq!(tex.height, MAX_FACES as u32);
        assert_eq!(tex.data.len(), FACE_DATA_W * MAX_FACES * 4);
    }

    #[test]
    fn dossier_text_texture_dimensions() {
        let mut analyzer = FaceDetectAnalyzer::new();
        let tex = analyzer.encode_dossier_text_texture(&[]);
        assert_eq!(tex.width, DOSSIER_TEX_W as u32);
        assert_eq!(tex.height, MAX_FACES as u32);
        assert_eq!(tex.data.len(), DOSSIER_TEX_W * MAX_FACES * 4);
    }
}
