/*{
    "DESCRIPTION": "Cymatics - Chladni plate and Faraday wave vibration pattern generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "mode_n", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Mode N"},
        {"NAME": "mode_m", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Mode M"},
        {"NAME": "plate_shape", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Shape (0=Square 1=Circle 2=Hex)"},
        {"NAME": "mix_ab", "TYPE": "float", "DEFAULT": 0.5, "MIN": -1.0, "MAX": 1.0, "LABEL": "Mode Mix A/B"},
        {"NAME": "resonance", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.05, "MAX": 1.0, "LABEL": "Resonance (Line Width)"},
        {"NAME": "vibration", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Vibration Amount"},
        {"NAME": "complexity", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Harmonic Complexity"},
        {"NAME": "sand_density", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0, "LABEL": "Sand Density"},
        {"NAME": "render_style", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Style (0=Sand 1=Water 2=Neon)"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.02, 0.02, 0.03, 1.0], "LABEL": "Background"},
        {"NAME": "line_color", "TYPE": "color", "DEFAULT": [0.95, 0.90, 0.78, 1.0], "LABEL": "Sand/Line Color"},
        {"NAME": "glow_amount", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Glow"},
        {"NAME": "evolve_speed", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0, "LABEL": "Evolution Speed"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 3.0, "LABEL": "Zoom"}
    ],
    "PHASE_INPUTS": [{"PARAM": "evolve_speed", "INDEX": 0}]
}*/

#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble;
    float audio_bpm; float audio_beat_phase; vec4 DATE;
    float PHASE_TIME_0; float PHASE_TIME_1; float PHASE_TIME_2; float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform UserParams {
    float mode_n; float mode_m; float plate_shape; float mix_ab;
    float resonance; float vibration; float complexity; float sand_density;
    float render_style; vec4 bg_color; vec4 line_color;
    float glow_amount; float evolve_speed; float zoom;
};

#define PI 3.14159265359
float audioSum() { return audio_level + audio_bass + audio_mid + audio_treble; }
float timeSum() { return PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3; }

// Bessel J0
float besselJ0(float x) {
    float ax = abs(x);
    if (ax < 8.0) {
        float y = x * x;
        float num = 57568490574.0+y*(-13362590354.0+y*(651619640.7+y*(-11214424.18+y*(77392.33017+y*(-184.9052456)))));
        float den = 57568490411.0+y*(1029532985.0+y*(9494680.718+y*(59272.64853+y*(267.8532712+y))));
        return num / den;
    }
    float z = 8.0/ax, y = z*z, xx = ax - 0.785398164;
    float p = 1.0+y*(-0.1098628627e-2+y*(0.2734510407e-4+y*(-0.2073370639e-5+y*0.2093887211e-6)));
    float q = -0.1562499995e-1+y*(0.1430488765e-3+y*(-0.6911147651e-5+y*(0.7621095161e-6-y*0.934935152e-7)));
    return sqrt(0.636619772/ax)*(p*cos(xx)-z*q*sin(xx));
}

// Bessel J1
float besselJ1(float x) {
    float ax = abs(x), sg = sign(x);
    if (ax < 8.0) {
        float y = x * x;
        float num = x*(72362614232.0+y*(-7895059235.0+y*(242396853.1+y*(-2972611.439+y*(15704.48260+y*(-30.16036606))))));
        float den = 144725228442.0+y*(2300535178.0+y*(18583304.74+y*(99447.43394+y*(376.9991397+y))));
        return num / den;
    }
    float z = 8.0/ax, y = z*z, xx = ax - 2.356194491;
    float p = 1.0+y*(0.183105e-2+y*(-0.3516396496e-4+y*(0.2457520174e-5+y*(-0.240337019e-6))));
    float q = 0.04687499995+y*(-0.2002690873e-3+y*(0.8449199096e-5+y*(-0.88228987e-6+y*0.105787412e-6)));
    return sg*sqrt(0.636619772/ax)*(p*cos(xx)-z*q*sin(xx));
}

// Bessel Jn via forward recurrence
float besselJn(int n, float x) {
    if (n == 0) return besselJ0(x);
    if (n == 1) return besselJ1(x);
    if (abs(x) < 1e-10) return 0.0;
    float j0 = besselJ0(x), j1 = besselJ1(x), jn = j0;
    for (int i = 1; i < n; i++) { jn = (2.0*float(i)/x)*j1 - j0; j0 = j1; j1 = jn; }
    return jn;
}

// Square plate: cos(nπx)cos(mπy) - cos(mπx)cos(nπy)
float chladniSquare(vec2 p, float n, float m, float ab) {
    float a = 0.5+0.5*ab, b = 0.5-0.5*ab;
    return a*cos(n*PI*p.x)*cos(m*PI*p.y) - b*cos(m*PI*p.x)*cos(n*PI*p.y);
}

// Circular plate: Bessel modes J_n(kr)cos(nθ)
float chladniCircle(vec2 p, float n, float m, float ab) {
    float r = length(p), theta = atan(p.y, p.x);
    int ni = int(floor(n)), mi = int(floor(m));
    float a = 0.5+0.5*ab, b = 0.5-0.5*ab;
    return a*besselJn(ni, m*PI*r)*cos(float(ni)*theta)
         + b*besselJn(mi, n*PI*r)*cos(float(mi)*theta);
}

// Hexagonal plate: 3 rotated square-plate superposition
float chladniHex(vec2 p, float n, float m, float ab) {
    float sum = 0.0;
    for (int i = 0; i < 3; i++) {
        float ang = float(i)*PI/3.0, c = cos(ang), s = sin(ang);
        sum += chladniSquare(vec2(c*p.x-s*p.y, s*p.x+c*p.y), n, m, ab);
    }
    return sum / 3.0;
}

float getDisp(vec2 st, int sh, float n, float m, float ab) {
    if (sh == 1) return chladniCircle(st, n, m, ab);
    if (sh == 2) return chladniHex(st, n, m, ab);
    return chladniSquare(st, n, m, ab);
}

float nodalLine(float d, float w) {
    float sh = mix(3.0, 40.0, 1.0-w);
    return exp(-d*d*sh*sh);
}

float hash21(vec2 p) { return fract(sin(dot(p, vec2(127.1,311.7)))*43758.5453); }

void main() {
    vec2 st = uv * 2.0 - 1.0;
    st.x *= RENDERSIZE.x / RENDERSIZE.y;
    st /= max(zoom, 0.3);

    float t = PHASE_TIME_0;
    float _unused = audioSum() + timeSum();

    // Audio-reactive mode parameters
    float n = floor(mode_n + audio_bass * 2.0);
    float m = floor(mode_m + audio_treble * 2.0);

    // Slowly morph modes over time
    float morph = sin(t * 0.5) * 0.5 + 0.5;
    float en = n + morph * complexity * 2.0;
    float em = m + (1.0 - morph) * complexity * 2.0;

    // Vibration oscillation
    float vib = sin(t * 6.2831 * (1.0 + audio_mid)) * vibration;

    int shape = int(floor(clamp(plate_shape, 0.0, 2.0)));
    float disp = getDisp(st, shape, en, em, mix_ab);

    // Add harmonic overtones
    if (complexity > 0.1) {
        disp += complexity * 0.4 * getDisp(st, shape, en+1.0, em+2.0, -mix_ab);
        disp += complexity * 0.2 * getDisp(st, shape, en*2.0, em, mix_ab*0.5);
    }

    disp *= (1.0 + vib);

    float nodal = nodalLine(abs(disp), resonance);
    float nodal_broad = nodalLine(abs(disp), resonance * 0.4);

    int style = int(floor(clamp(render_style, 0.0, 2.0)));
    vec3 color = vec3(0.0);

    if (style == 0) {
        // SAND style
        vec3 sand = line_color.rgb;
        float grain = hash21(floor(st * RENDERSIZE.y * 0.5)) * 0.15 + 0.85;
        float mask = smoothstep(1.0-sand_density, 1.0, nodal) * grain;
        float halo = nodal_broad * glow_amount * 0.3;
        color = bg_color.rgb*(1.0-mask-halo) + sand*mask + sand*0.5*halo;
        color += sand * abs(nodal - nodal_broad) * 0.4;
    } else if (style == 1) {
        // WATER style
        float eps = 0.005;
        float dx = (getDisp(st+vec2(eps,0), shape, en, em, mix_ab) - disp) / eps;
        float dy = (getDisp(st+vec2(0,eps), shape, en, em, mix_ab) - disp) / eps;
        vec3 norm = normalize(vec3(-dx*0.05, -dy*0.05, 1.0));
        vec3 ld = normalize(vec3(0.3, 0.5, 1.0));
        float diff = max(dot(norm, ld), 0.0);
        float spec = pow(max(dot(norm, normalize(ld+vec3(0,0,1))), 0.0), 64.0);
        vec3 deep = vec3(0.02, 0.05, 0.15);
        vec3 shallow = vec3(0.1, 0.3, 0.5);
        vec3 water = mix(deep, shallow, smoothstep(-0.5, 0.5, disp));
        float caustic = pow(abs(dx+dy)*0.3, 2.0) * 3.0;
        color = water*(0.4+diff*0.6) + vec3(0.8,0.9,1.0)*spec*0.8
              + vec3(0.4,0.7,1.0)*caustic*glow_amount + vec3(0.2,0.4,0.6)*nodal*0.15;
    } else {
        // NEON style
        float hue = disp * 2.0 + t * 0.3;
        vec3 neon = 0.5 + 0.5*cos(6.2831*(hue + vec3(0.0, 0.33, 0.67)));
        color = bg_color.rgb + neon*nodal_broad*glow_amount*0.5
              + line_color.rgb*nodal*1.5 + neon*pow(nodal,4.0)*2.0*glow_amount;
    }

    // Plate boundary
    float bnd = 1.0;
    if (shape == 1) {
        bnd = 1.0 - smoothstep(0.95, 1.0, length(st));
    } else if (shape == 0) {
        bnd = 1.0 - smoothstep(0.95, 1.0, max(abs(st.x), abs(st.y)));
    } else {
        vec2 ap = abs(st);
        bnd = 1.0 - smoothstep(0.93, 0.98, max(ap.x, ap.x*0.5+ap.y*0.866));
    }

    color = mix(bg_color.rgb * 0.3, color * bnd, bnd);
    fragColor = vec4(clamp(color, 0.0, 1.0), 1.0);
}
