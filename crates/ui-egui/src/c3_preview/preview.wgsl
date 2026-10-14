struct View { rotation: vec4<f32>, center_scale: vec4<f32>, camera: vec4<f32>, options: vec4<f32>, axes:vec4<f32> }
@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var tex: texture_2d<f32>;
@group(1) @binding(1) var samp: sampler;
struct In { @location(0) position: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) uv: vec2<f32> }
struct Out { @builtin(position) position: vec4<f32>, @location(0) normal: vec3<f32>, @location(1) uv: vec2<f32> }
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
    o.normal = rotate(basis(v.normal));
    o.uv = vec2(v.uv.x,select(v.uv.y,1.0-v.uv.y,view.options.w>0.5));
    return o;
}
fn color(v: Out) -> vec4<f32> {
    var c = textureSample(tex,samp,v.uv);
    if view.options.y>0.5 {
        let n = normalize(v.normal);
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
