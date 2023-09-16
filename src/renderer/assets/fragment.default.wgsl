// Very simple algorithm. Explained great here - https://www.youtube.com/watch?v=kbKtFN71Lfs&ab_channel=Numberphile
// You can basically do whatever transformations you want, which makes it really fun.
// The shader is projecting points, then splatting them on the screen.
// Great demo which uses this effect - https://www.youtube.com/watch?v=xLN3mTRlugs&ab_channel=AssemblyTV

alias iv4 = vec4<i32>;alias iv3 = vec3<i32>;alias iv2 = vec2<i32>;alias uv4 = vec4<u32>;alias uv3 = vec3<u32>;alias uv2 = vec2<u32>;alias v4 = vec4<f32>;alias v3 = vec3<f32>;alias v2 = vec2<f32>;alias m2 = mat2x2<f32>;alias m3 = mat3x3<f32>;alias m4 = mat4x4<f32>;
#define global var<private>
var<workgroup> texCoords: array<array<float2, 16>, 16>;
#define Frame time.frame
#define T (time.elapsed*float(custom.Paused > 0.5) + 5.) 
#define Tb (time.elapsed) 
#define pi acos(-1.)
#define tau (acos(-1.)*2.)
global R: v2;
global U: v2;
global seed: uint;
global muv: v2;
fn rot(a: float)-> m2{ return m2(cos(a), -sin(a),sin(a), cos(a));}
fn rotX(a: float) -> m3{
    let r = rot(a); return m3(1.,0.,0.,0.,r[0][0],r[0][1],0.,r[1][0],r[1][1]);
}
fn rotY(a: float) -> m3{
    let r = rot(a); return m3(r[0][0],0.,r[0][1],0.,1.,0.,r[1][0],0.,r[1][1]);
}
fn rotZ(a: float) -> m3{
    let r = rot(a); return m3(r[0][0],r[0][1],0.,r[1][0],r[1][1],0.,0.,0.,1.);
}
fn hash_u(_a: uint) -> uint{
    var a: uint = _a;
    a = a ^ (a >> 16u);
    a *= 0x7feb352du;
    a ^= a >> 15u;
    a *= 0x846ca68bu;
    a ^= a >> 16u;
    return a;
}
fn hash_f() -> float{ var s = hash_u(seed); seed = s;return ( float( s ) / float( 0xffffffffu ) ); }
fn hash_v2() -> v2{ return v2(hash_f(), hash_f()); }
fn hash_v3() -> v3{ return v3(hash_f(), hash_f(), hash_f()); }
fn hash_v4() -> v4{ return v4(hash_f(), hash_f(), hash_f(), hash_f()); }

fn sample_disk() -> v2{
    let r = hash_v2();
    return v2(sin(r.x*tau),cos(r.x*tau))*sqrt(r.y);
}


#define COL_CNT 4
global kCols: array<v3, COL_CNT> = array<v3, COL_CNT>( 
     vec3(1.0,1.0,1.0), vec3(0.8,0.4,0.7),
     vec3(1.5,1.5,1.5), vec3(1.5,1.5,1.5)
);

fn mix_cols(_idx: float)->v3{
    let idx = _idx%1.;
    var cols_idx = int(idx*float(COL_CNT));
    var fract_idx = fract(idx*float(COL_CNT));
    fract_idx = smoothstep(0.,1.,fract_idx);
    //return oklab_mix( kCols[cols_idx], kCols[(cols_idx + 1)%COL_CNT], fract_idx );
    return mix( kCols[cols_idx], kCols[(cols_idx + 1)%COL_CNT], fract_idx );
}


#storage hist_atomic array<atomic<u32>>

#storage h array<u32>

fn projParticle(_p: v3) -> v3{
    var p = _p; // you still can't modify args in wgsl???
    // p *= rotY(muv.x*2.);
    
    p += sin(v3(3.,2.,1.) + Tb*0.7)*0.1;

    p *= rotZ(Tb*0.6 + sin(Tb)*0.6);
    // p *= rotX(muv.y*2.);
    p *= rotY(Tb*0.3+ sin(Tb*0.8)*0.35);

    p = p/mix(1.0,(dot(p,p)+4.),custom.C);
    p.z += 0.6;
    p /= p.z*(.2 + custom.B*1.);
    p.z = _p.z;
    p.x /= R.x/R.y;
    return p;
}

fn t_spherical(p: v3, rad: float, offs: v3) -> v3{
    return p/(dot(p,p)*rad + offs);
}

#workgroup_count Splat 64 64 2
@compute @workgroup_size(256, 1,1)
fn Splat(@builtin(global_invocation_id) id: uint3) {
    let Ru = uint2(textureDimensions(screen));
    if (id.x >= Ru.x || id.y >= Ru.y) { return; }
    R = v2(Ru); U = v2(id.xy); muv = (v2(mouse.pos) - 0.5*R)/R.y;
    
    seed = hash_u(id.x + hash_u(Ru.x*id.y*200u)*20u + hash_u(id.x)*250u + hash_u(id.z)*250u );
    seed = hash_u(seed);

    let particleIdx = id.x;
    
    let iters = 1060;
    
    var t = T - hash_f()*1./30.;

    var env = t + sin(t)*0.5;
    var envb = sin(t*0.45);

    var p = hash_v3();

    let focusDist = (custom.DOF_Focal_Dist*2. - 1.)*5.;
    let dofFac = 1./vec2(R.x/R.y,1.)*custom.DOF_Amount;
    for(var i = 0; i < iters; i++){
        let r = hash_f();
        if(r<.1){
            p = p/(dot(p,p)*1.5 + 0.);
            if(hash_f() < 0.5){
                // p.z = -p.z;
                p = -p;
            }

        } else if(r< 0.2){
            p = p/(dot(p,p)*(1. - custom.D*8.) + 0.1); 
        } else {
            p = p * rotZ(0.4 + (custom.A*2. - 1.) + sin(Tb*0.7)*0.1);
            p += 0.4;
        }
        
        var q = projParticle(p);
        var k = q.xy;

        k += sample_disk()*abs(q.z - focusDist)*0.05*dofFac;
        
        let uv = k.xy/2. + 0.5;
        let cc = uv2(uv.xy*R.xy);
        let idx = cc.x + Ru.x * cc.y;
        if ( 
            // q.z > -20.
            uv.x > 0. && uv.x < 1. 
            && uv.y > 0. && uv.y < 1. 
            && idx < uint(Ru.x*Ru.y)
            ){     
            atomicAdd(&hist_atomic[idx],1u);
            atomicAdd(&hist_atomic[idx + Ru.x*Ru.y],uint(r*100.));
        }
    }
}

@compute @workgroup_size(16, 16)
fn main_image(@builtin(global_invocation_id) id: uint3) {
    let res = uint2(textureDimensions(screen));
    if (id.x >= res.x || id.y >= res.y) { return; }

    R = v2(res);
    U = v2(id.xy);
    let U = float2(float(id.x) + .5, float(res.y - id.y) - .5);

    
    let hist_id = id.x + uint(R.x) * id.y;

    var col = float(atomicLoad(&hist_atomic[hist_id]))*vec3(3.0);
    
    var col_pal = float(atomicLoad(&hist_atomic[hist_id + res.x*res.y]))*vec3(3.0);
    
    kCols[1] *= rotY(1. - 0.347*log(col.x*1.)*4. + col_pal.x*0.0001);
    // tonemap
    let sc = 124452.7;
    col = log( col*0.3)/ log(sc);


    col = col*mix_cols(col.x*1.);
    // col += 0.01;
    // col = mix_cols(log(col_pal.x)*0.5);

    //col = smoothstep(v3(0.),v3(1.),col*mix_cols(col.x*1.));

    col = pow((col), float3(1./0.45454545));
    textureStore(screen, int2(id.xy), float4(col, 1.));
    

    // clear flame
    // hist[id.x + uint(R.x) * id.y] = 0;
    atomicStore(&hist_atomic[hist_id],0u);
    atomicStore(&hist_atomic[hist_id + res.x*res.y],0u);
}
