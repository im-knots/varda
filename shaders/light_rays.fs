/*{
    "DESCRIPTION": "Light Rays - volumetric light shafts streaming from bright areas",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "threshold", "LABEL": "Threshold", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "intensity", "LABEL": "Intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "ray_length", "LABEL": "Length", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "decay", "LABEL": "Decay", "TYPE": "float", "DEFAULT": 0.97, "MIN": 0.9, "MAX": 1.0},
        {"NAME": "center_x", "LABEL": "Light X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_y", "LABEL": "Light Y", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "tint", "LABEL": "Tint", "TYPE": "color", "DEFAULT": [1.0, 0.95, 0.85, 1.0]},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PASSES": [
        {"TARGET": "bright", "WIDTH": "$WIDTH/2", "HEIGHT": "$HEIGHT/2"},
        {"TARGET": "rays", "WIDTH": "$WIDTH/2", "HEIGHT": "$HEIGHT/2"}
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
layout(set = 0, binding = 3) uniform texture2D bright;
layout(set = 0, binding = 4) uniform texture2D rays;

layout(set = 0, binding = 5) uniform UserParams {
    float threshold;
    float intensity;
    float ray_length;
    float decay;
    float center_x;
    float center_y;
    vec4 tint;
    float amount;
};

const int TAPS = 48;

void main() {
    if (PASSINDEX == 0) {
        vec3 c = max(texture(sampler2D(inputImage, texSampler), uv).rgb, 0.0);
        float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
        fragColor = vec4(c * max(l - threshold, 0.0), 1.0);
        return;
    }

    if (PASSINDEX == 1) {
        // March from the pixel toward the light, gathering the bright pass with
        // a falloff per tap (GPU Gems 3, ch. 13). A per-pixel start offset
        // turns banding into fine noise.
        vec2 step = (uv - vec2(center_x, center_y)) * ray_length / float(TAPS);
        float jitter = fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715))));
        vec2 coord = uv - step * jitter;
        float falloff = 1.0;
        vec3 sum = vec3(0.0);
        for (int i = 0; i < TAPS; i++) {
            sum += texture(sampler2D(bright, texSampler), coord).rgb * falloff;
            falloff *= decay;
            coord -= step;
        }
        fragColor = vec4(sum / float(TAPS), 1.0);
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec3 shafts = texture(sampler2D(rays, texSampler), uv).rgb;
    fragColor = vec4(max(src.rgb + shafts * tint.rgb * intensity * amount, 0.0), src.a);
}
