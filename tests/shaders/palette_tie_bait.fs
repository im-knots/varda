/*{
    "DESCRIPTION": "Test fixture: two candidate colours tied at the palette selection boundary.",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "tie", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Tie"}
    ]
}*/

// Chroma Flow's auto palette picks anchors by greedy farthest-point over a 5x5
// grid of samples. That is a *selection*, and a selection has a cliff in it:
// when two candidates are near-tied, an arbitrarily small change swaps which one
// wins, and the anchor for that slot jumps to a completely different colour.
//
// Real content reaches that cliff only by luck. `dull_skull` does on the DX12
// backend and does not on Metal, which is why the lurch test failed on Windows
// and could not be reproduced anywhere else, and why two attempted fixes were
// evaluated against a test with nothing to detect.
//
// This fixture removes the luck. It renders flat blocks on the exact 5x5 grid
// `extractAutoPalette` samples, and `tie` slides two candidates through the
// point where they swap:
//
//   centre (2,2)  mid grey, always the first pick
//   (0,0)         blue,   |D - grey|^2 = 0.50 + (d - 0.5)^2
//   (4,4)         yellow, |E - grey|^2 = 0.25 + 2 * (e - 0.5)^2
//   everything else  the same grey, distance zero, never picked
//
// The two intensities move in opposite directions so those distances cross at
// `tie = 0.5`. With Palette Size at 2 there is exactly one slot after the
// centre, so at the crossing the palette's only anchor flips between blue and
// yellow, and the whole frame regrades in a single step. That is the failure
// under study, made deterministic and available on every backend.

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform UserParams {
    float tie;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // -1 at tie=0, +1 at tie=1, so the crossing sits at tie=0.5.
    float s = tie * 2.0 - 1.0;

    // Amplitudes chosen so the two squared distances to grey are equal at s=0.
    // Blue moves twice as fast as yellow purely so the crossing is not
    // degenerate: they meet at a point rather than sliding along together.
    vec3 grey = vec3(0.5);
    vec3 blue = vec3(0.0, 0.0, 0.90 + 0.02 * s);
    float y = 0.9528 - 0.01 * s;
    vec3 yellow = vec3(y, y, 0.0);

    ivec2 cell = ivec2(floor(clamp(uv, 0.0, 0.999) * 5.0));
    vec3 c = grey;
    if (cell == ivec2(0, 0)) c = blue;
    if (cell == ivec2(4, 4)) c = yellow;

    fragColor = vec4(c, 1.0);
}
