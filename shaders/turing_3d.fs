/*{
    "DESCRIPTION": "3D Turing Pattern - Ray marched volumetric reaction-diffusion",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "3D"],
    "INPUTS": [
        {"NAME": "rotation_speed", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0, "LABEL": "Rotation Speed"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 2.0, "LABEL": "Zoom"},
        {"NAME": "feed_rate", "TYPE": "float", "DEFAULT": 0.055, "MIN": 0.01, "MAX": 0.1, "LABEL": "Feed Rate (F)"},
        {"NAME": "kill_rate", "TYPE": "float", "DEFAULT": 0.062, "MIN": 0.03, "MAX": 0.09, "LABEL": "Kill Rate (k)"},
        {"NAME": "pattern_scale", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 2.0, "LABEL": "Pattern Scale"},
        {"NAME": "evolution", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0, "LABEL": "Evolution Speed"},
        {"NAME": "density", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.1, "MAX": 1.0, "LABEL": "Density"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.1, 0.05, 0.15, 1.0], "LABEL": "Deep Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.8, 0.3, 0.5, 1.0], "LABEL": "Mid Color"},
        {"NAME": "color3", "TYPE": "color", "DEFAULT": [1.0, 0.9, 0.7, 1.0], "LABEL": "Bright Color"},
        {"NAME": "glow_intensity", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Glow"}
    ]
}*/

#version 450
layout(location = 0) out vec4 fragColor;

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
};

layout(set = 0, binding = 1) uniform UserParams {
    float rotation_speed;
    float zoom;
    float feed_rate;
    float kill_rate;
    float pattern_scale;
    float evolution;
    float density;
    vec4 color1;
    vec4 color2;
    vec4 color3;
    float glow_intensity;
};

// Stable 3D noise - no time dependency in the hash
float hash31(vec3 p) {
    p = fract(p * vec3(0.1031, 0.1030, 0.0973));
    p += dot(p, p.yxz + 33.33);
    return fract((p.x + p.y) * p.z);
}

float noise3d(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    f = f * f * f * (f * (f * 6.0 - 15.0) + 10.0); // Quintic interpolation (smoother)

    return mix(
        mix(mix(hash31(i), hash31(i + vec3(1,0,0)), f.x),
            mix(hash31(i + vec3(0,1,0)), hash31(i + vec3(1,1,0)), f.x), f.y),
        mix(mix(hash31(i + vec3(0,0,1)), hash31(i + vec3(1,0,1)), f.x),
            mix(hash31(i + vec3(0,1,1)), hash31(i + vec3(1,1,1)), f.x), f.y), f.z);
}

// Turing-like pattern using feed/kill to control shape
float turingPattern3D(vec3 p, float time) {
    float scale = 5.0 / max(pattern_scale, 0.1);
    vec3 sp = p * scale;

    // Slow, smooth time evolution (no flickering)
    float slowTime = time * 0.02;

    // Domain warping amount influenced by feed rate (more feed = more warping)
    float warpAmount = 0.5 + feed_rate * 8.0;

    // Static warp field (no time in the warp itself - prevents flicker)
    float warpX = noise3d(sp * 0.7 + vec3(0.0, 5.3, 2.7));
    float warpY = noise3d(sp * 0.7 + vec3(3.1, 0.0, 8.5));
    float warpZ = noise3d(sp * 0.7 + vec3(7.7, 2.3, 0.0));
    vec3 warped = sp + vec3(warpX, warpY, warpZ) * warpAmount;

    // Slowly evolving pattern (time only affects position, not noise seed)
    vec3 evolving = warped + vec3(slowTime, slowTime * 0.7, slowTime * 0.5);

    // Two scales for Turing-like spots
    float n1 = noise3d(evolving);
    float n2 = noise3d(evolving * 2.1 + 100.0);

    // Mix based on kill rate (higher kill = more fine detail)
    float detailMix = kill_rate * 12.0;
    float pattern = mix(n1, n2, clamp(detailMix - 0.5, 0.0, 0.5));

    // Threshold controlled by feed/kill balance (classic Turing parameter space)
    float threshold = 0.45 + (feed_rate - kill_rate) * 3.0;
    float edge = 0.12 + kill_rate * 0.5; // Sharper edges with higher kill

    pattern = smoothstep(threshold - edge, threshold + edge, pattern);

    return pattern;
}

// Rotation matrix
mat3 rotateY(float angle) {
    float c = cos(angle), s = sin(angle);
    return mat3(c, 0, s, 0, 1, 0, -s, 0, c);
}

mat3 rotateX(float angle) {
    float c = cos(angle), s = sin(angle);
    return mat3(1, 0, 0, 0, c, -s, 0, s, c);
}

// SDF for bounding sphere
float sdSphere(vec3 p, float r) {
    return length(p) - r;
}

void main() {
    vec2 uv = (gl_FragCoord.xy - 0.5 * RENDERSIZE) / min(RENDERSIZE.x, RENDERSIZE.y);
    
    float time = TIME * evolution;
    
    // Camera setup
    float camDist = 3.0 / max(zoom, 0.1);
    float rotAngle = TIME * rotation_speed * 0.5;
    
    vec3 ro = vec3(0.0, 0.0, camDist); // Ray origin
    ro = rotateY(rotAngle) * rotateX(sin(TIME * 0.3) * 0.3) * ro;
    
    vec3 target = vec3(0.0);
    vec3 forward = normalize(target - ro);
    vec3 right = normalize(cross(vec3(0, 1, 0), forward));
    vec3 up = cross(forward, right);
    
    vec3 rd = normalize(forward + uv.x * right + uv.y * up); // Ray direction

    // Ray march through volume
    float boundRadius = 1.5;
    vec3 color = vec3(0.0);
    float alpha = 0.0;

    // Find intersection with bounding sphere
    float b = dot(ro, rd);
    float c = dot(ro, ro) - boundRadius * boundRadius;
    float disc = b * b - c;

    if (disc > 0.0) {
        float sqrtDisc = sqrt(disc);
        float t0 = max(-b - sqrtDisc, 0.0);
        float t1 = -b + sqrtDisc;

        // Optimized ray marching - 40 steps is plenty for smooth volumes
        const int MAX_STEPS = 40;
        float stepSize = (t1 - t0) / float(MAX_STEPS);
        float t = t0 + stepSize * 0.5; // Start at half-step for better sampling

        vec3 accumColor = vec3(0.0);
        float accumAlpha = 0.0;
        float invRange = 1.0 / (t1 - t0);

        for (int i = 0; i < MAX_STEPS; i++) {
            if (accumAlpha > 0.95) break; // Early exit

            vec3 p = ro + rd * t;
            float pLen = length(p);

            // Density falloff near edges of sphere
            float sphereFalloff = 1.0 - smoothstep(0.7, 1.4, pLen);

            if (sphereFalloff > 0.01) {
                // Get Turing pattern density at this point
                float d = turingPattern3D(p, time) * sphereFalloff * density * 2.0;

                if (d > 0.02) {
                    // Color gradient based on density
                    vec3 sampleColor = mix(color1.rgb, color2.rgb, smoothstep(0.0, 0.5, d));
                    sampleColor = mix(sampleColor, color3.rgb, smoothstep(0.5, 1.0, d));

                    // Depth-based shading
                    float depth = (t - t0) * invRange;
                    sampleColor *= 0.75 + 0.25 * (1.0 - depth);

                    // Accumulate with front-to-back compositing
                    float sampleAlpha = min(d * stepSize * 4.0, 1.0);
                    accumColor += sampleColor * sampleAlpha * (1.0 - accumAlpha);
                    accumAlpha += sampleAlpha * (1.0 - accumAlpha);
                }
            }

            t += stepSize;
        }

        color = accumColor;
        alpha = accumAlpha;
    }

    // Background gradient (stable) — only where there's no pattern
    vec3 bgColor = mix(color1.rgb * 0.2, color1.rgb * 0.05, uv.y * 0.5 + 0.5);
    color = mix(bgColor, color, alpha);
    float outAlpha = clamp(alpha + 0.05, 0.0, 1.0); // slight base from bg gradient, full where pattern exists

    // Simple additive glow on top (very stable - no edge detection)
    if (glow_intensity > 0.01 && alpha > 0.1) {
        vec3 glowColor = mix(color2.rgb, color3.rgb, 0.6);
        color += glowColor * alpha * glow_intensity * 0.15;
    }

    // Audio reactivity - subtle pulse with bass
    color *= 1.0 + audio_bass * 0.2;

    fragColor = vec4(color * outAlpha, outAlpha);
}

