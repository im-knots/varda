/*{
    "DESCRIPTION": "Plasma Globe - raymarched electrical arcs writhing inside a glowing plasma sphere, seen through a reflective outer shell",
    "CREDIT": "Varda VJ (ported from 'Plasma Globe' by nimitz, https://www.shadertoy.com/view/XsjXRm)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "time_scale", "TYPE": "float", "DEFAULT": 1.1, "MIN": 0.0, "MAX": 3.0, "LABEL": "Time Scale"},
        {"NAME": "num_rays", "TYPE": "float", "DEFAULT": 13.0, "MIN": 4.0, "MAX": 30.0, "LABEL": "Arc Rays"},
        {"NAME": "arc_glow_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Arc Glow Intensity"},
        {"NAME": "flow_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Shell Flow Speed"},
        {"NAME": "sphere_reflect_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Shell Reflection Intensity"},
        {"NAME": "look_at", "TYPE": "point2D", "DEFAULT": [0.5, 0.5], "LABEL": "Look At"},
        {"NAME": "tint", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Tint"}
    ],
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0, "SCALE": 1.0}]
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
    float speed;
    float time_scale;
    float num_rays;
    float arc_glow_intensity;
    float flow_speed;
    float sphere_reflect_intensity;
    vec2 look_at;
    vec4 tint;
};

// Looks best with around 25 rays; NUM_RAYS_MAX bounds the fixed-size loop
// that `num_rays` dynamically truncates via an early break (naga/shaderc
// prefer compile-time loop bounds over a fully dynamic `for` condition).
#define NUM_RAYS_MAX 30
#define VOLUMETRIC_STEPS 19
#define MAX_ITER 35
#define FAR 6.

// Original used `#define time iTime*1.1`; PHASE_TIME_0 is the smooth
// phase accumulator driven by the `speed` PHASE_INPUTS param, and
// `time_scale` exposes the original's baked-in 1.1 multiplier as a
// tunable (defaulting to 1.1 so the untouched default reproduces the
// original speed exactly).
#define time (PHASE_TIME_0 * time_scale)

mat2 mm2(in float a){float c = cos(a), s = sin(a);return mat2(c,-s,s,c);}

// noise(x): 1D value noise. Originally a textureLod lookup into
// iChannel0 (a bound noise texture); Varda generators are self-contained
// with no bound input image channel, so this is replaced with a fully
// procedural hash-based value noise of the same [0,1] range/continuity.
float hash1(float n) { return fract(sin(n)*43758.5453); }
float noise(in float x) {
    float i = floor(x);
    float f = fract(x);
    f = f*f*(3.0-2.0*f);
    return mix(hash1(i), hash1(i+1.0), f);
}

float hash( float n ){return fract(sin(n)*43758.5453);}

// noise(vec3 p): 3D value noise. Originally a textureLod lookup into a
// precomputed 256x256 tiled noise texture (iChannel0); replaced with a
// fully procedural trilinearly-interpolated hash value noise (same
// [0,1] range/continuity as the original) since flow() and vmarch()
// below depend on smooth, continuous 3D noise rather than a bare hash.
float hash13(vec3 p) {
    p = fract(p * vec3(443.897, 441.423, 437.195));
    p += dot(p, p.yzx + 19.19);
    return fract((p.x + p.y) * p.z);
}
float noise(in vec3 p)
{
    vec3 ip = floor(p);
    vec3 fp = fract(p);
    fp = fp*fp*(3.0-2.0*fp);
    float n000 = hash13(ip);
    float n100 = hash13(ip+vec3(1.0,0.0,0.0));
    float n010 = hash13(ip+vec3(0.0,1.0,0.0));
    float n110 = hash13(ip+vec3(1.0,1.0,0.0));
    float n001 = hash13(ip+vec3(0.0,0.0,1.0));
    float n101 = hash13(ip+vec3(1.0,0.0,1.0));
    float n011 = hash13(ip+vec3(0.0,1.0,1.0));
    float n111 = hash13(ip+vec3(1.0,1.0,1.0));
    return mix(mix(mix(n000,n100,fp.x),mix(n010,n110,fp.x),fp.y),
               mix(mix(n001,n101,fp.x),mix(n011,n111,fp.x),fp.y), fp.z);
}

mat3 m3 = mat3( 0.00,  0.80,  0.60,
              -0.80,  0.36, -0.48,
              -0.60, -0.48,  0.64 );

float flow(in vec3 p, in float t)
{
	float z=2.;
	float rz = 0.;
	vec3 bp = p;
	for (float i= 1.;i < 5.;i++ )
	{
		p += time*.1;
		rz+= (sin(noise(p+t*0.8)*6.)*0.5+0.5) /z;
		p = mix(bp,p,0.6);
		z *= 2.;
		p *= 2.01;
        p*= m3;
	}
	return rz;
}

float sins(in float x)
{
 	float rz = 0.;
    float z = 2.;
    for (float i= 0.;i < 3.;i++ )
	{
        rz += abs(fract(x*1.4)-0.5)/z;
        x *= 1.3;
        z *= 1.15;
        x -= time*.65*z;
    }
    return rz;
}

float segm( vec3 p, vec3 a, vec3 b)
{
    vec3 pa = p - a;
	vec3 ba = b - a;
	float h = clamp( dot(pa,ba)/dot(ba,ba), 0.0, 1. );
	return length( pa - ba*h )*.5;
}

vec3 path(in float i, in float d)
{
    vec3 en = vec3(0.,0.,1.);
    float sns2 = sins(d+i*0.5)*0.22;
    float sns = sins(d+i*.6)*0.21;
    en.xz *= mm2((hash(i*10.569)-.5)*6.2+sns2);
    en.xy *= mm2((hash(i*4.732)-.5)*6.2+sns);
    return en;
}

vec2 map(vec3 p, float i)
{
	float lp = length(p);
    vec3 bg = vec3(0.);
    vec3 en = path(i,lp);

    float ins = smoothstep(0.11,.46,lp);
    float outs = .15+smoothstep(.0,.15,abs(lp-1.));
    p *= ins*outs;
    float id = ins*outs;

    float rz = segm(p, bg, en)-0.011;
    return vec2(rz,id);
}

float march(in vec3 ro, in vec3 rd, in float startf, in float maxd, in float j)
{
	float precis = 0.001;
    float h=0.5;
    float d = startf;
    for( int i=0; i<MAX_ITER; i++ )
    {
        if( abs(h)<precis||d>maxd ) break;
        d += h*1.2;
	    float res = map(ro+rd*d, j).x;
        h = res;
    }
	return d;
}

vec3 vmarch(in vec3 ro, in vec3 rd, in float j, in vec3 orig)
{
    vec3 p = ro;
    vec2 r = vec2(0.);
    vec3 sum = vec3(0);
    float w = 0.;
    for( int i=0; i<VOLUMETRIC_STEPS; i++ )
    {
        r = map(p,j);
        p += rd*.03;
        float lp = length(p);

        vec3 col = sin(vec3(1.05,2.5,1.52)*3.94+r.y)*.85+0.4;
        col.rgb *= smoothstep(.0,.015,-r.x);
        col *= smoothstep(0.04,.2,abs(lp-1.1));
        col *= smoothstep(0.1,.34,lp);
        sum += abs(col)*5. * (1.2-noise(lp*2.+j*13.+time*5.)*1.1) / (log(distance(p,orig)-2.)+.75);
    }
    return sum;
}

vec2 iSphere2(in vec3 ro, in vec3 rd)
{
    vec3 oc = ro;
    float b = dot(oc, rd);
    float c = dot(oc,oc) - 1.;
    float h = b*b - c;
    if(h <0.0) return vec2(-1.);
    else return vec2((-b - sqrt(h)), (-b + sqrt(h)));
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward), the opposite of
    // Shadertoy's bottom-left/y-up fragCoord convention. The original's
    // p.y is positive toward the top of the screen (matching "up" in
    // world/camera space, since rd.y follows p.y directly with no other
    // sign flip before the camera basis is built), so we flip y here to
    // preserve that "up is up" orientation for the plasma sphere and its
    // electrical arcs.
    vec2 p = vec2(uv.x, 1.0 - uv.y) - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // iMouse -> look_at point2D. Like star_nest.fs's look_at, um here
    // doesn't address a screen-space position -- it feeds two abstract
    // rotation-angle offsets (um.x*6, um.y*6) added to the camera's
    // xz/xy spin angles, so there's no "up" semantic to preserve by
    // flipping. Left unflipped; DEFAULT [0.5,0.5] maps to um=(0,0),
    // i.e. no extra offset beyond the automatic time-driven spin,
    // which is the calmest/neutral default for a VJ tool.
    vec2 um = look_at - 0.5;

    vec3 ro = vec3(0.,0.,5.);
    vec3 rd = normalize(vec3(p*.7,-1.5));
    mat2 mx = mm2(time*.4+um.x*6.);
    mat2 my = mm2(time*0.3+um.y*6.);
    ro.xz *= mx;rd.xz *= mx;
    ro.xy *= my;rd.xy *= my;

    vec3 bro = ro;
    vec3 brd = rd;

    vec3 col = vec3(0.0125,0.,0.025);
    int numRaysInt = int(clamp(num_rays, 1.0, float(NUM_RAYS_MAX)));
    for (int ji = 0; ji < NUM_RAYS_MAX; ji++)
    {
        if (ji >= numRaysInt) break;
        float j = float(ji) + 1.0;
        ro = bro;
        rd = brd;
        mat2 mm = mm2((time*0.1+((j+1.)*5.1))*j*0.25);
        ro.xy *= mm;rd.xy *= mm;
        ro.xz *= mm;rd.xz *= mm;
        float rz = march(ro,rd,2.5,FAR,j);
		if ( rz >= FAR) continue;
    	vec3 pos = ro+rz*rd;
        // arc_glow_intensity scales vmarch's whole `abs(col)*5.` volumetric
        // brightness term (a linear scale, so applying it to the return
        // value here is equivalent, and keeps the background ambient
        // `col` initializer above untouched by the intensity control).
    	col = max(col, vmarch(pos,rd,j, bro) * arc_glow_intensity);
    }

    ro = bro;
    rd = brd;
    vec2 sph = iSphere2(ro,rd);

    if (sph.x > 0.)
    {
        vec3 pos = ro+rd*sph.x;
        vec3 pos2 = ro+rd*sph.y;
        vec3 rf = reflect( rd, pos );
        vec3 rf2 = reflect( rd, pos2 );
        // flow_speed independently scales the time argument fed into
        // flow(), separate from the overall speed/time_scale driving
        // the rest of the scene.
        float flowT = time * flow_speed;
        float nz = (-log(abs(flow(rf*1.2,flowT)-.01)));
        float nz2 = (-log(abs(flow(rf2*1.2,-flowT)-.01)));
        col += (0.1*nz*nz* vec3(0.12,0.12,.5) + 0.05*nz2*nz2*vec3(0.55,0.2,.55))*0.8*sphere_reflect_intensity;
    }

    col = col*1.3*tint.rgb;
    fragColor = vec4(col, 1.0);
}
