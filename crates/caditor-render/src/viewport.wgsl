struct View {
    rotation_projection: mat4x4<f32>,
    forward_near: vec4<f32>,
    viewport: vec4<f32>,
    pick_transform: vec4<f32>,
    light: vec4<f32>,
}

struct Grid {
    origin_extent: vec4<f32>,
    axis_u_spacing: vec4<f32>,
    axis_v_fade: vec4<f32>,
    color: vec4<f32>,
}

struct MeshPlacement {
    offset: vec4<f32>,
    faces_columns: vec4<u32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var<uniform> grid: Grid;
@group(1) @binding(1) var face_styles: texture_2d<u32>;
@group(1) @binding(2) var<uniform> mesh: MeshPlacement;

const CULLED: vec4<f32> = vec4<f32>(0.0, 0.0, 2.0, 1.0);
const ORTHOGRAPHIC_DEPTH_BIAS: f32 = 0.03;
const HALF_DEPTH_RANGE: f32 = 0.5;
const BEHIND: u32 = 0u;
const DASH_PERIOD_POINTS: f32 = 10.0;
const DASH_DRAWN_FRACTION: f32 = 0.6;

struct Varyings {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) pick: u32,
    @location(2) depth: f32,
    @location(3) local: vec2<f32>,
    @location(4) @interpolate(flat) diameter: f32,
    @location(5) relative: vec3<f32>,
    @location(6) normal: vec3<f32>,
    @location(7) dash_points: f32,
}

struct PickOutput {
    @location(0) id: u32,
    @location(1) depth: u32,
}

fn view_depth(position: vec3<f32>) -> f32 {
    return dot(position, view.forward_near.xyz);
}

fn to_clip(position: vec3<f32>) -> vec4<f32> {
    return view.rotation_projection * vec4<f32>(position, 1.0);
}

fn is_orthographic() -> bool {
    return view.viewport.w > 0.5;
}

fn biased_depth(clip: vec4<f32>, depth_bias: f32) -> f32 {
    if is_orthographic() {
        return clip.z + (depth_bias - 1.0) * ORTHOGRAPHIC_DEPTH_BIAS * clip.w;
    }
    return clip.z * depth_bias;
}

fn layered_depth(clip: vec4<f32>, depth_bias: f32, in_front: u32) -> f32 {
    let depth = biased_depth(clip, depth_bias) * HALF_DEPTH_RANGE;
    return select(depth, depth + HALF_DEPTH_RANGE * clip.w, in_front != BEHIND);
}

fn finish(clip: vec4<f32>, depth_bias: f32, in_front: u32) -> vec4<f32> {
    return vec4<f32>(
        clip.x * view.pick_transform.x + view.pick_transform.z * clip.w,
        clip.y * view.pick_transform.y + view.pick_transform.w * clip.w,
        layered_depth(clip, depth_bias, in_front),
        clip.w,
    );
}

fn toward_eye(relative: vec3<f32>) -> vec3<f32> {
    if is_orthographic() {
        return -view.forward_near.xyz;
    }
    return normalize(-relative);
}

fn pixels_per_point() -> f32 {
    return view.viewport.z;
}

fn pixels_to_ndc(pixels: vec2<f32>) -> vec2<f32> {
    return pixels * 2.0 / view.viewport.xy;
}

fn ndc_to_pixels(clip: vec4<f32>) -> vec2<f32> {
    return clip.xy / clip.w * view.viewport.xy * 0.5;
}

fn empty_varyings() -> Varyings {
    var out: Varyings;
    out.position = CULLED;
    out.color = vec4<f32>(0.0);
    out.pick = 0u;
    out.depth = 0.0;
    out.local = vec2<f32>(0.0);
    out.diameter = 0.0;
    out.relative = vec3<f32>(0.0);
    out.normal = vec3<f32>(0.0);
    out.dash_points = -1.0;
    return out;
}

fn quad_corner(index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    return corners[index % 6u];
}

struct LineInstance {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) width: f32,
    @location(4) pick: u32,
    @location(5) depth_bias: f32,
    @location(6) along: f32,
    @location(7) in_front: u32,
}

@vertex
fn vs_line(@builtin(vertex_index) vertex: u32, line: LineInstance) -> Varyings {
    let near = view.forward_near.w * 1.01;
    var start = line.start;
    var end = line.end;
    let start_depth = view_depth(start);
    let end_depth = view_depth(end);
    if start_depth < near && end_depth < near {
        return empty_varyings();
    }
    if start_depth < near {
        start = mix(start, end, (near - start_depth) / (end_depth - start_depth));
    }
    if end_depth < near {
        end = mix(end, start, (near - end_depth) / (start_depth - end_depth));
    }

    let start_clip = to_clip(start);
    let end_clip = to_clip(end);
    let along_pixels = ndc_to_pixels(end_clip) - ndc_to_pixels(start_clip);
    var direction = vec2<f32>(1.0, 0.0);
    if length(along_pixels) > 1e-6 {
        direction = normalize(along_pixels);
    }
    let normal = vec2<f32>(-direction.y, direction.x);

    let corner = quad_corner(vertex);
    let at_end = corner.x > 0.0;
    let half_width = line.width * pixels_per_point() * 0.5;
    let offset = normal * corner.y * half_width;
    var clip = select(start_clip, end_clip, at_end);
    clip = vec4<f32>(clip.xy + pixels_to_ndc(offset) * clip.w, clip.zw);

    var out = empty_varyings();
    out.position = finish(clip, line.depth_bias, line.in_front);
    out.color = line.color;
    out.pick = line.pick;
    out.depth = select(view_depth(start), view_depth(end), at_end);
    if line.along >= 0.0 {
        let clipped_length = distance(start, end);
        let points_per_unit = length(along_pixels) / (max(clipped_length, 1e-12) * pixels_per_point());
        let along_start = line.along + distance(line.start, start);
        out.dash_points = (along_start + select(0.0, clipped_length, at_end)) * points_per_unit;
    }
    return out;
}

struct MarkerInstance {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) diameter: f32,
    @location(3) pick: u32,
    @location(4) depth_bias: f32,
    @location(5) in_front: u32,
}

@vertex
fn vs_marker(@builtin(vertex_index) vertex: u32, marker: MarkerInstance) -> Varyings {
    let depth = view_depth(marker.position);
    if depth < view.forward_near.w * 1.01 {
        return empty_varyings();
    }
    let corner = quad_corner(vertex);
    let diameter = marker.diameter * pixels_per_point();
    let radius = diameter * 0.5 + 1.0;
    let center = to_clip(marker.position);
    let clip = vec4<f32>(center.xy + pixels_to_ndc(corner * radius) * center.w, center.zw);

    var out = empty_varyings();
    out.position = finish(clip, marker.depth_bias, marker.in_front);
    out.color = marker.color;
    out.pick = marker.pick;
    out.depth = depth;
    out.local = corner * radius;
    out.diameter = diameter;
    return out;
}

struct FillVertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pick: u32,
    @location(3) depth_bias: f32,
    @location(4) in_front: u32,
}

@vertex
fn vs_fill(fill: FillVertex) -> Varyings {
    var out = empty_varyings();
    out.position = finish(to_clip(fill.position), fill.depth_bias, fill.in_front);
    out.color = fill.color;
    out.pick = fill.pick;
    out.depth = view_depth(fill.position);
    return out;
}

struct MeshVertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) face: u32,
}

fn unpack_color(packed: u32) -> vec4<f32> {
    let channels = vec4<u32>(packed, packed >> 8u, packed >> 16u, packed >> 24u) & vec4<u32>(255u);
    return vec4<f32>(channels) / 255.0;
}

@vertex
fn vs_mesh(vertex: MeshVertex) -> Varyings {
    let relative = vertex.position + mesh.offset.xyz;
    let face = min(vertex.face, max(mesh.faces_columns.x, 1u) - 1u);
    let columns = max(mesh.faces_columns.y, 1u);
    let style = textureLoad(face_styles, vec2<u32>(face % columns, face / columns), 0);

    var out = empty_varyings();
    out.position = finish(to_clip(relative), 1.0, BEHIND);
    out.color = unpack_color(style.x);
    out.pick = style.y;
    out.depth = view_depth(relative);
    out.relative = relative;
    out.normal = vertex.normal;
    return out;
}

@vertex
fn vs_grid(@builtin(vertex_index) vertex: u32) -> Varyings {
    let local = quad_corner(vertex) * grid.origin_extent.w;
    let position = grid.origin_extent.xyz
        + grid.axis_u_spacing.xyz * local.x
        + grid.axis_v_fade.xyz * local.y;

    var out = empty_varyings();
    out.position = finish(to_clip(position), 1.0, BEHIND);
    out.color = grid.color;
    out.relative = position;
    out.local = local;
    return out;
}

@fragment
fn fs_color(in: Varyings) -> @location(0) vec4<f32> {
    return in.color;
}

@fragment
fn fs_line(in: Varyings) -> @location(0) vec4<f32> {
    if in.dash_points >= 0.0 && fract(in.dash_points / DASH_PERIOD_POINTS) > DASH_DRAWN_FRACTION {
        discard;
    }
    return in.color;
}

const AMBIENT: f32 = 0.3;
const KEY_LIGHT: f32 = 0.55;
const HEADLIGHT: f32 = 0.2;
const SPECULAR: f32 = 0.12;
const SHININESS: f32 = 40.0;

@fragment
fn fs_mesh(in: Varyings) -> @location(0) vec4<f32> {
    let eye = toward_eye(in.relative);
    var normal = in.normal;
    if dot(normal, normal) < 1e-12 {
        normal = eye;
    }
    normal = normalize(normal);
    if dot(normal, eye) < 0.0 {
        normal = -normal;
    }
    let toward_light = view.light.xyz;
    let key = max(dot(normal, toward_light), 0.0);
    let head = max(dot(normal, eye), 0.0);
    let halfway = normalize(toward_light + eye);
    let shine = pow(max(dot(normal, halfway), 0.0), SHININESS) * SPECULAR;
    let shade = AMBIENT + KEY_LIGHT * key + HEADLIGHT * head;
    return vec4<f32>(in.color.rgb * shade + vec3<f32>(shine), in.color.a);
}

fn marker_coverage(in: Varyings) -> f32 {
    return clamp(in.diameter * 0.5 - length(in.local) + 0.5, 0.0, 1.0);
}

@fragment
fn fs_marker(in: Varyings) -> @location(0) vec4<f32> {
    let coverage = marker_coverage(in);
    if coverage <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

fn grid_level(coordinate: vec2<f32>, spacing: f32) -> f32 {
    let cell = coordinate / spacing;
    let cells_per_point = max(fwidth(cell) * pixels_per_point(), vec2<f32>(1e-6));
    let points_to_line = abs(fract(cell - 0.5) - 0.5) / cells_per_point;
    let line = 1.0 - clamp(min(points_to_line.x, points_to_line.y), 0.0, 1.0);
    let cell_points = 1.0 / max(cells_per_point.x, cells_per_point.y);
    return line * smoothstep(4.0, 16.0, cell_points);
}

@fragment
fn fs_grid(in: Varyings) -> @location(0) vec4<f32> {
    let spacing = grid.axis_u_spacing.w;
    let minor = grid_level(in.local, spacing) * 0.45;
    let major = grid_level(in.local, spacing * 10.0) * 0.75;
    let coarse = grid_level(in.local, spacing * 100.0);
    let fade_end = grid.axis_v_fade.w;
    let distance_fade = 1.0 - smoothstep(fade_end * 0.25, fade_end, length(in.relative));
    let alpha = max(minor, max(major, coarse)) * distance_fade * in.color.a;
    if alpha <= 0.002 {
        discard;
    }
    return vec4<f32>(in.color.rgb, alpha);
}

@fragment
fn fs_pick(in: Varyings) -> PickOutput {
    if in.pick == 0u {
        discard;
    }
    return PickOutput(in.pick, bitcast<u32>(in.depth));
}

@fragment
fn fs_mesh_pick(in: Varyings) -> PickOutput {
    return PickOutput(in.pick, bitcast<u32>(in.depth));
}

@fragment
fn fs_marker_pick(in: Varyings) -> PickOutput {
    if in.pick == 0u || marker_coverage(in) <= 0.0 {
        discard;
    }
    return PickOutput(in.pick, bitcast<u32>(in.depth));
}
