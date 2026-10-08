/*{
    "DESCRIPTION": "Paint - oil paint look from an edge-preserving anisotropic Kuwahara filter",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 6.0, "MIN": 2.0, "MAX": 12.0},
        {"NAME": "sharpness", "LABEL": "Sharpness", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 18.0},
        {"NAME": "anisotropy", "LABEL": "Anisotropy", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "render_scale", "LABEL": "Render Scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 1.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PASSES": [
        {"TARGET": "encoded", "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {"TARGET": "tensor", "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {"TARGET": "flow", "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {"TARGET": "painted", "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"}
    ]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform texture2D encoded;
layout(set = 0, binding = 4) uniform texture2D tensor;
layout(set = 0, binding = 5) uniform texture2D flow;
layout(set = 0, binding = 6) uniform texture2D painted;

layout(set = 0, binding = 7) uniform UserParams {
    float radius;
    float sharpness;
    float anisotropy;
    float render_scale;
    float amount;
};

// The filter works on sRGB-encoded values so edges in the shadows count as
// much as they look. Above 1.0 the curve continues linearly.
vec3 encode(vec3 c) {
    c = max(c, 0.0);
    vec3 lo = mix(c * 12.92, 1.055 * pow(min(c, 1.0), vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
    return mix(lo, 1.0 + (c - 1.0) * (1.055 / 2.4), step(1.0, c));
}

vec3 decode(vec3 e) {
    vec3 lo = mix(e / 12.92, pow((clamp(e, 0.0, 1.0) + 0.055) / 1.055, vec3(2.4)), step(0.04045, e));
    return mix(lo, 1.0 + (e - 1.0) * (2.4 / 1.055), step(1.0, e));
}

vec3 inputAt(vec2 p) { return texture(sampler2D(encoded, texSampler), p).rgb; }

void main() {
    // Effect passes see the deck's RENDERSIZE, so take the pass size from a buffer.
    vec2 texel = 1.0 / vec2(textureSize(sampler2D(tensor, texSampler), 0));

    if (PASSINDEX == 0) {
        fragColor = vec4(encode(texture(sampler2D(inputImage, texSampler), uv).rgb), 1.0);
        return;
    }

    if (PASSINDEX == 1) {
        if (anisotropy <= 0.0) {
            fragColor = vec4(0.0);
            return;
        }
        // Sobel gradients of the color, summed over channels into the
        // structure tensor (E, F, G).
        vec3 tl = inputAt(uv + texel * vec2(-1, -1));
        vec3 t = inputAt(uv + texel * vec2(0, -1));
        vec3 tr = inputAt(uv + texel * vec2(1, -1));
        vec3 l = inputAt(uv + texel * vec2(-1, 0));
        vec3 r = inputAt(uv + texel * vec2(1, 0));
        vec3 bl = inputAt(uv + texel * vec2(-1, 1));
        vec3 b = inputAt(uv + texel * vec2(0, 1));
        vec3 br = inputAt(uv + texel * vec2(1, 1));
        vec3 gx = (tr + 2.0 * r + br - tl - 2.0 * l - bl) * 0.25;
        vec3 gy = (bl + 2.0 * b + br - tl - 2.0 * t - tr) * 0.25;
        fragColor = vec4(dot(gx, gx), dot(gy, gy), dot(gx, gy), 0.0);
        return;
    }

    if (PASSINDEX == 2) {
        if (anisotropy <= 0.0) {
            fragColor = vec4(1.0, 0.0, 0.0, 0.0);
            return;
        }
        // Smooth the tensor with a small Gaussian (bilinear taps between texels),
        // then take the local edge direction and how strongly it dominates.
        vec3 s = texture(sampler2D(tensor, texSampler), uv).xyz * 0.25;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(1.5, 0.0)).xyz * 0.125;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(-1.5, 0.0)).xyz * 0.125;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(0.0, 1.5)).xyz * 0.125;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(0.0, -1.5)).xyz * 0.125;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(1.5, 1.5)).xyz * 0.0625;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(-1.5, 1.5)).xyz * 0.0625;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(1.5, -1.5)).xyz * 0.0625;
        s += texture(sampler2D(tensor, texSampler), uv + texel * vec2(-1.5, -1.5)).xyz * 0.0625;
        float E = s.x, G = s.y, F = s.z;
        float root = sqrt((E - G) * (E - G) + 4.0 * F * F);
        float l1 = 0.5 * (E + G + root);
        float l2 = 0.5 * (E + G - root);
        vec2 dir = vec2(l1 - E, -F);
        dir = dot(dir, dir) > 1e-12 ? normalize(dir) : vec2(0.0, 1.0);
        float aniso = l1 + l2 > 1e-8 ? (l1 - l2) / (l1 + l2) : 0.0;
        fragColor = vec4(dir, aniso, 0.0);
        return;
    }

    if (PASSINDEX == 3) {
        vec4 f = texture(sampler2D(flow, texSampler), uv);
        vec2 dir = dot(f.xy, f.xy) > 1e-6 ? normalize(f.xy) : vec2(1.0, 0.0);
        float A = f.z * anisotropy;
        // Stretch the kernel along the edge and squeeze it across.
        float rad = radius * render_scale;
        vec2 axes = rad * vec2((1.0 + A), 1.0 / (1.0 + A));
        mat2 toPixels = mat2(dir.x, dir.y, -dir.y, dir.x) * mat2(axes.x, 0.0, 0.0, axes.y);

        // Polynomial sector weights (Kyprianidis): sectors 0,2,4,6 in the "a"
        // vectors, 1,3,5,7 in the "b" vectors, one vec4 per color channel.
        const float ZETA_K = 2.0;
        float zeta = ZETA_K / rad;
        const float CROSS = 0.58;
        float eta = (zeta + cos(CROSS)) / (sin(CROSS) * sin(CROSS));

        vec3 c0 = inputAt(uv);
        vec4 wA = vec4(1.0 / 8.0), wB = vec4(1.0 / 8.0);
        vec4 mRA = c0.r * wA, mGA = c0.g * wA, mBA = c0.b * wA;
        vec4 mRB = c0.r * wB, mGB = c0.g * wB, mBB = c0.b * wB;
        vec4 sA = dot(c0, c0) * wA, sB = dot(c0, c0) * wB;

        int rings = int(clamp(ceil(rad / 4.0), 2.0, 3.0));
        for (int i = 1; i <= 3; i++) {
            if (i > rings) break;
            float rho = float(i) / float(rings);
            int taps = 8 * i;
            float stepAngle = 6.28318530718 / float(taps);
            float ringWeight = exp(-3.125 * rho * rho);
            // Walk the ring by repeated rotation, offset half a step on odd rings.
            vec2 rot = vec2(cos(stepAngle), sin(stepAngle));
            float start = 0.5 * float(i & 1) * stepAngle;
            vec2 v = rho * vec2(cos(start), sin(start));
            for (int j = 0; j < 24; j++) {
                if (j >= taps) break;
                if (j > 0) v = vec2(v.x * rot.x - v.y * rot.y, v.x * rot.y + v.y * rot.x);
                vec3 c = inputAt(uv + toPixels * v * texel);

                vec2 vxy = zeta - eta * v * v;
                vec4 wa = max(vec4(v.y, -v.x, -v.y, v.x) + vxy.xyxy, 0.0);
                vec2 u = 0.70710678 * vec2(v.x - v.y, v.x + v.y);
                vec2 uxy = zeta - eta * u * u;
                vec4 wb = max(vec4(u.y, -u.x, -u.y, u.x) + uxy.xyxy, 0.0);
                wa *= wa;
                wb *= wb;
                float g = ringWeight / max(dot(wa, vec4(1.0)) + dot(wb, vec4(1.0)), 1e-6);
                wa *= g;
                wb *= g;

                wA += wa; wB += wb;
                mRA += c.r * wa; mGA += c.g * wa; mBA += c.b * wa;
                mRB += c.r * wb; mGB += c.g * wb; mBB += c.b * wb;
                float cc = dot(c, c);
                sA += cc * wa; sB += cc * wb;
            }
        }

        // Each sector's mean and variance; flat sectors get the most say.
        mRA /= wA; mGA /= wA; mBA /= wA;
        mRB /= wB; mGB /= wB; mBB /= wB;
        vec4 varA = abs(sA / wA - (mRA * mRA + mGA * mGA + mBA * mBA));
        vec4 varB = abs(sB / wB - (mRB * mRB + mGB * mGB + mBB * mBB));
        vec4 kA = 1.0 / (1.0 + pow(min(255.0 * varA, vec4(1000.0)), vec4(0.5 * sharpness)));
        vec4 kB = 1.0 / (1.0 + pow(min(255.0 * varB, vec4(1000.0)), vec4(0.5 * sharpness)));
        float kSum = max(dot(kA, vec4(1.0)) + dot(kB, vec4(1.0)), 1e-20);
        vec3 col = vec3(dot(kA, mRA) + dot(kB, mRB),
                        dot(kA, mGA) + dot(kB, mGB),
                        dot(kA, mBA) + dot(kB, mBB)) / kSum;
        fragColor = vec4(decode(col), 1.0);
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec3 paint = texture(sampler2D(painted, texSampler), uv).rgb;
    fragColor = vec4(max(mix(src.rgb, paint, amount), 0.0), src.a);
}
