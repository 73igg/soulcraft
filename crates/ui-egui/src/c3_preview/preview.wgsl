struct View { rotation: vec4<f32>, center_scale: vec4<f32>, camera: vec4<f32>, options: vec4<f32>, axes:vec4<f32> }
@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
struct Material { info:vec4<f32>, data:array<vec4<f32>,4>, steps01:vec4<f32>, step2:vec4<f32> }
@group(2) @binding(0) var mix_tex:texture_2d<f32>;
@group(2) @binding(1) var mix_samp:sampler;
@group(2) @binding(2) var<uniform> material:Material;
@group(2) @binding(3) var third_tex:texture_2d<f32>;
@group(2) @binding(4) var fourth_tex:texture_2d<f32>;
struct In { @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32>, @location(3) color:vec4<f32>, @location(4) uv1:vec2<f32> }
struct Out { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color:vec4<f32>, @location(3) uv1:vec2<f32>, @location(4) to_eye:vec3<f32> }
fn sample_primary(uv:vec2<f32>)->vec4<f32> {return textureSampleLevel(tex,samp,uv,0.0);}
fn sample_second(uv:vec2<f32>)->vec4<f32> {return textureSampleLevel(mix_tex,mix_samp,uv,0.0);}
fn sample_third(uv:vec2<f32>)->vec4<f32> {return textureSampleLevel(third_tex,mix_samp,uv,0.0);}
fn safe_normal(value:vec3<f32>)->vec3<f32> {
    let length2=dot(value,value);
    return value*inverseSqrt(max(length2,0.00000000000000000001));
}
fn union_alpha(a:f32,b:f32)->f32 {return 1.0-(1.0-a)*(1.0-b);}
fn sampled(v:Out)->vec4<f32> {
    let mode=u32(material.info.x);
    let step=material.steps01.xy;
    let second_step=material.steps01.zw;
    let third_step=material.step2.xy;
    let original=v.uv-step;
    let secondary=v.uv1+step;
    let p=material.data[0];
    let a=sample_primary(v.uv);
    let b=sample_second(v.uv);
    let tint=v.color;
    var result=a*tint;
    switch mode {
        case 4u: {result=a*b*tint*p.x;}
        case 7u: {result=vec4((a.rgb*p.x+b.rgb*p.y)*tint.rgb,a.a*tint.a);}
        case 8u: {result=mix(b,a,material.info.z)*tint;}
        case 9u: {result=vec4(2.0*(a.rgb+b.rgb-vec3(0.5))*tint.rgb,a.a*tint.a);}
        case 10u: {let light=sample_second(v.uv1);result=vec4((a.rgb*light.rgb+vec3(light.a))*tint.rgb,a.a*tint.a);}
        case 11u: {result=vec4((a.rgb+b.rgb-a.rgb*b.rgb)*tint.rgb,a.a*tint.a);}
        case 13u: {result=vec4(mix(a.rgb,b.rgb,b.a)*tint.rgb,a.a*tint.a);}
        case 15u: {result=vec4(a.rgb*sample_second(v.uv1).rgb*tint.rgb*p.w,a.a*tint.a);}
        case 19u: {if a.a<0.0314 {discard;} result=tint;}
        case 16u: {
            let reflection=sample_second(safe_normal(v.normal).xy);
            result=vec4(a.rgb*reflection.rgb*p.x,a.a*tint.a);
        }
        case 23u: {
            if material.info.w>0.5 {
                let base=sample_primary(original);
                let mask=1.0-sample_second(original).a;
                let flow=clamp(sample_second(v.uv).rgb+vec3(mask),vec3(0.0),vec3(1.0));
                let gray=dot(base.rgb,vec3(0.3,0.59,0.11));
                result=vec4(2.0*gray*flow*tint.rgb,base.a*tint.a);
            }
        }
        case 36u: {
            let noise=sample_third(original+second_step).xy;
            let threshold=sample_third(secondary+noise).a;
            let difference=material.info.z-threshold;
            var weight=select(-1.0,1.0,difference>=0.0);
            if p.x!=0.0 {weight=clamp(difference/p.x,-1.0,1.0);}
            result=mix(sample_primary(original),sample_second(original),(weight+1.0)*0.5);
            if result.a==0.0 {discard;}
        }
        case 28u: {
            let base=sample_primary(original);let overlay=sample_second(secondary);
            result=vec4((base.rgb+overlay.rgb)*tint.rgb,base.a*overlay.a*tint.a);
        }
        case 30u: {
            let base=sample_primary(original);
            let layer1=sample_second(original+second_step);
            let layer2=sample_third(original+third_step);
            let layer3=textureSampleLevel(fourth_tex,mix_samp,secondary,0.0);
            result=vec4(base.rgb+layer1.rgb*layer1.a+layer2.rgb*layer2.a+layer3.rgb,base.a*layer3.a);
        }
        case 31u,32u: {
            let distortion=sample_third(original+third_step).xy;
            let cutoff=sample_second(original+second_step+distortion).r;
            let base=sample_primary(secondary);
            let distance=tint.a-cutoff;
            if distance<0.0 || (material.data[2].x!=0.0 && base.a<0.1) {discard;}
            var coverage=select(0.0,1.0,distance>0.0);
            if p.x!=0.0 {coverage=clamp(distance/p.x,0.0,1.0);}
            var rgb=base.rgb;
            if mode==32u {
                if coverage<=p.y {rgb=material.data[1].xyz;}
                else if coverage<p.z && p.z!=p.y {rgb=mix(base.rgb,material.data[1].xyz,(p.z-coverage)/(p.z-p.y));}
            }
            result=vec4(rgb,base.a*coverage);
        }
        case 33u: {if result.a<0.1 {discard;}result.a=1.0;}
        case 34u: {
            if a.a<0.05 {discard;}
            result=a;
            if tint.a<1.0 {
                let dimensions=vec2<f32>(textureDimensions(mix_tex));
                let coordinate=fract((dimensions*second_step+p.xy*v.position.xy)/dimensions);
                let blend=sample_second(coordinate);
                var rgb=mix(blend.rgb,a.rgb,tint.a);
                if p.w!=0.0 {rgb=a.rgb+(1.0-tint.a)*blend.rgb;}
                result=vec4(rgb,a.a*(tint.a+blend.a*(1.0-tint.a)));
            }
            result.a*=material.info.z;
        }
        case 35u: {
            let base=sample_primary(original);
            if base.a<0.0314 {discard;}
            let mask=sample_second(original).a;
            let fluid=sample_second(v.uv1+second_step).rgb+sample_second(v.uv1+third_step).rgb;
            result=vec4((base.rgb+mask*fluid)*tint.rgb,base.a*tint.a);
        }
        case 37u,38u: {
            let normal=safe_normal(v.normal);let eye=safe_normal(v.to_eye);
            let rim=clamp(1.0-abs(dot(normal,eye)),0.0,1.0);
            var fresnel=clamp(p.w*rim,0.0,1.0);
            if mode==38u {fresnel=pow(max(rim,0.000001),p.w);}
            let base=sample_primary(original);let overlay=sample_second(secondary);
            var source=overlay;
            if material.data[2].y==0.0 {source+=base;}
            let alpha=fresnel+(1.0-fresnel)*source.a;
            if (material.data[2].x!=0.0 && alpha<0.1) || (material.data[2].y!=0.0 && base.a<0.0314) {discard;}
            result=vec4(mix(source.rgb,p.xyz,fresnel)*tint.rgb,alpha*tint.a);
        }
        case 39u,40u: {
            var a0=a*tint;var b0=sample_second(v.uv1+second_step)*tint;var c0=sample_third(v.uv1+third_step)*tint;
            let selected=select(0xffffffffu,u32(max(p.x,0.0)),p.x>=0.0 && p.x==floor(p.x));let alpha=material.info.z;
            if selected==0u || selected>select(1u,2u,mode==40u) {a0.a*=alpha;}
            if selected==1u || selected>select(1u,2u,mode==40u) {b0.a*=alpha;}
            if selected==2u || selected>2u {c0.a*=alpha;}
            let operation=select(0xffffffffu,u32(max(p.y,0.0)),p.y>=0.0 && p.y==floor(p.y));let ar=a0.rgb*a0.a;let br=b0.rgb*b0.a;let cr=c0.rgb*c0.a;
            if mode==39u {
                if operation==0u {result=vec4(ar+br,a0.a);}
                else if operation==1u {result=vec4(br+ar*(1.0-b0.a),b0.a);}
                else if operation==2u {result=vec4(ar+br,0.0);}
                else {result=vec4(br+ar*(1.0-b0.a),union_alpha(a0.a,b0.a));}
            } else {
                switch operation {
                    case 0u: {result=vec4(ar+br+cr,a0.a);}
                    case 1u: {result=vec4((ar+br)*(1.0-c0.a)+cr,union_alpha(a0.a,b0.a));}
                    case 2u: {result=vec4(cr+br+ar*(1.0-b0.a),b0.a);}
                    case 3u: {result=vec4(cr+br*(1.0-c0.a)+ar*(1.0-b0.a)*(1.0-c0.a),union_alpha(a0.a,b0.a));}
                    case 4u: {result=vec4(ar+br+cr,0.0);}
                    case 5u: {result=vec4(cr+(br+ar)*(1.0-c0.a),c0.a);}
                    case 6u: {result=vec4(cr+br+ar*(1.0-b0.a),union_alpha(a0.a,b0.a));}
                    default: {result=vec4(cr+(br+ar*(1.0-b0.a))*(1.0-c0.a),union_alpha(union_alpha(a0.a,b0.a),c0.a));}
                }
            }
        }
        default: {}
    }
    return clamp(result,vec4(0.0),vec4(1.0));
}
fn rotate(v: vec3<f32>) -> vec3<f32> { let q = view.rotation; return v + 2.0 * cross(q.xyz, cross(q.xyz,v) + q.w*v); }
fn basis(p:vec3<f32>)->vec3<f32> {
    if view.axes.x<0.5 {return vec3(p.x,-p.z,-p.y);}
    if view.axes.x<1.5 {return vec3(p.x,p.z,-p.y);}
    if view.axes.x<2.5 {return p;}
    return vec3(p.x,-p.y,p.z);
}
@vertex fn vs(v: In) -> Out {
    // Eudemons uses negative Z for height; other client conventions are selectable.
    let local = (v.position-view.center_scale.xyz)*view.center_scale.w;
    let p = rotate(basis(local));
    let z = max(0.01, view.camera.x-p.z);
    // Perspective projection with w=z and WebGPU's [0,1] depth range.
    let near = 0.01; let far = 100.0;
    var o: Out;
    o.position = vec4((p.x+view.camera.y)*view.camera.w/view.options.x,(p.y+view.camera.z)*view.camera.w,far/(far-near)*z-far*near/(far-near),z);
    o.color=v.color;
    o.uv1=vec2(v.uv1.x,select(v.uv1.y,1.0-v.uv1.y,view.options.w>0.5));
    o.to_eye=vec3(-view.camera.y,-view.camera.z,view.camera.x)-p;
    o.normal = rotate(basis(v.normal));
    o.uv = vec2(v.uv.x,select(v.uv.y,1.0-v.uv.y,view.options.w>0.5));
    return o;
}
fn color(v: Out) -> vec4<f32> {
    var c = sampled(v);
    if view.options.y>0.5 {
        let n = safe_normal(v.normal);
        let light = 0.45 + 0.55*abs(dot(n,normalize(vec3(0.3,0.6,1.0))));
        c = vec4(c.rgb*light,c.a);
    }
    if view.options.z>1.5 { c.a=1.0; }
    return vec4(c.rgb*c.a,c.a);
}
@fragment fn opaque(v: Out) -> @location(0) vec4<f32> {
    let c=color(v);
    let threshold=select(0.995,0.5,view.options.z>0.5);
    if c.a<threshold { discard; }
    return vec4(c.rgb/max(c.a,0.0001),1.0);
}
@fragment fn transparent(v: Out) -> @location(0) vec4<f32> {
    if view.options.z>0.5 { discard; }
    let c=color(v);
    if c.a>=0.995 || c.a<0.003 { discard; }
    return c;
}

@fragment fn client(v:Out)->@location(0) vec4<f32> {
    var c=sampled(v);
    if view.options.y>0.5 {
        let n=safe_normal(v.normal);
        c=vec4(c.rgb*(0.45+0.55*abs(dot(n,normalize(vec3(0.3,0.6,1.0))))),c.a);
    }
    if view.options.z>1.5 {c.a=1.0;}
    else if view.options.z>0.5 {if c.a<0.5 {discard;} c.a=1.0;}
    return c;
}
