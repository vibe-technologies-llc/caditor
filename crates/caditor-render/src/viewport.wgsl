struct View {
    rotation_projection: mat4x4<f32>,
    forward_near: vec4<f32>,
    viewport: vec4<f32>,
    pick_transform: vec4<f32>,
    light: vec4<f32>,
    fill_light: vec4<f32>,
    anchor: vec4<f32>,
    reflection_across: vec4<f32>,
    reflection_along: vec4<f32>,
    section: vec4<f32>,
    section_planes: array<vec4<f32>, 6>,
    section_hatches: array<vec4<f32>, 6>,
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
    turn_x: vec4<f32>,
    turn_y: vec4<f32>,
    turn_z: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(1) @binding(0) var<uniform> grid: Grid;
@group(1) @binding(1) var face_styles: texture_2d<u32>;
@group(1) @binding(2) var<uniform> mesh: MeshPlacement;

const CULLED: vec4<f32> = vec4<f32>(0.0, 0.0, 2.0, 1.0);
const ORTHOGRAPHIC_DEPTH_BIAS: f32 = 0.03;
const HALF_DEPTH_RANGE: f32 = 0.5;
const GRID_DEPTH_BIAS: f32 = 0.99998;
const BEHIND: u32 = 0u;
const IN_FRONT: u32 = 1u;
const SECTIONED: u32 = 2u;
const CAPPABLE: u32 = 4u;
const SOLID_WHERE_SEEN: u32 = 8u;
const MAX_SECTION_PLANES: u32 = 6u;
const FACE_SLOPE_BIAS: f32 = 2.0;
const CAP_DEPTH_BIAS: f32 = 1.0002;
const CAP_SHADE: f32 = 0.7;
const HATCH_SHADE: f32 = 0.25;
const HATCH_WIDTH_POINTS: f32 = 1.0;
const DASH_PERIOD_POINTS: f32 = 10.0;
const DASH_DRAWN_FRACTION: f32 = 0.6;
const STROKE_FRINGE_PIXELS: f32 = 1.0;
const OPAQUE_ALPHA: f32 = 0.999;
const SRGB_LINEAR_SLOPE: f32 = 12.92;
const SRGB_DECODED_KNEE: f32 = 0.04045;
const SRGB_ENCODED_KNEE: f32 = 0.0031308;
const SRGB_OFFSET: f32 = 0.055;
const SRGB_EXPONENT: f32 = 2.4;

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
    @location(8) stroke: vec3<f32>,
    @location(9) @interpolate(flat) stroke_extent: vec3<f32>,
    @location(10) @interpolate(flat) sectioned: u32,
}

struct PickOutput {
    @location(0) id: u32,
    @location(1) depth: u32,
}

fn to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / SRGB_LINEAR_SLOPE;
    let high = pow((max(color, vec3<f32>(0.0)) + SRGB_OFFSET) / (1.0 + SRGB_OFFSET), vec3<f32>(SRGB_EXPONENT));
    return select(high, low, color <= vec3<f32>(SRGB_DECODED_KNEE));
}

fn to_srgb(color: vec3<f32>) -> vec3<f32> {
    let clamped = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = clamped * SRGB_LINEAR_SLOPE;
    let high = (1.0 + SRGB_OFFSET) * pow(clamped, vec3<f32>(1.0 / SRGB_EXPONENT)) - SRGB_OFFSET;
    return select(high, low, clamped <= vec3<f32>(SRGB_ENCODED_KNEE));
}

fn from_anchor(position: vec3<f32>) -> vec3<f32> {
    return position + view.anchor.xyz;
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
    out.stroke = vec3<f32>(0.0);
    out.stroke_extent = vec3<f32>(0.0);
    out.sectioned = 0u;
    return out;
}

fn section_count() -> u32 {
    return min(u32(max(view.section.x, 0.0)), MAX_SECTION_PLANES);
}

fn beyond(plane: vec4<f32>, relative: vec3<f32>) -> f32 {
    return dot(plane.xyz, relative) - plane.w;
}

fn is_cut_away(relative: vec3<f32>) -> bool {
    var cut = false;
    for (var index = 0u; index < section_count(); index += 1u) {
        cut = cut || beyond(view.section_planes[index], relative) > view.section.y;
    }
    return cut;
}

fn is_cut(in: Varyings) -> bool {
    return in.sectioned != 0u && is_cut_away(in.relative);
}

fn quad_corner(index: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 1.0),
    );
    return corners[index % 4u];
}

struct LineInstance {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) width: f32,
    @location(4) pick: u32,
    @location(5) depth_bias: f32,
    @location(6) along: f32,
    @location(7) flags: u32,
}

struct Segment {
    start: vec3<f32>,
    end: vec3<f32>,
    start_clip: vec4<f32>,
    end_clip: vec4<f32>,
    along_pixels: vec2<f32>,
}

fn near_limit() -> f32 {
    return view.forward_near.w * 1.01;
}

fn is_before_near(start: vec3<f32>, end: vec3<f32>) -> bool {
    return view_depth(start) < near_limit() && view_depth(end) < near_limit();
}

fn clipped_segment(head: vec3<f32>, tail: vec3<f32>) -> Segment {
    let near = near_limit();
    var start = head;
    var end = tail;
    let start_depth = view_depth(start);
    let end_depth = view_depth(end);
    if start_depth < near {
        start = mix(start, end, (near - start_depth) / (end_depth - start_depth));
    }
    if end_depth < near {
        end = mix(end, start, (near - end_depth) / (start_depth - end_depth));
    }
    let start_clip = to_clip(start);
    let end_clip = to_clip(end);
    return Segment(start, end, start_clip, end_clip, ndc_to_pixels(end_clip) - ndc_to_pixels(start_clip));
}

fn segment_direction(segment: Segment) -> vec2<f32> {
    if length(segment.along_pixels) > 1e-6 {
        return normalize(segment.along_pixels);
    }
    return vec2<f32>(1.0, 0.0);
}

fn finishes_strokes() -> bool {
    return view.fill_light.w > 0.5;
}

struct Stroke {
    width: f32,
    depth_bias: f32,
    in_front: u32,
    capped: bool,
}

fn stroke_reach(stroke: Stroke) -> vec2<f32> {
    let half_width = stroke.width * pixels_per_point() * 0.5;
    if !finishes_strokes() {
        return vec2<f32>(0.0, half_width);
    }
    let cap = select(0.0, half_width, stroke.capped);
    return vec2<f32>(cap + STROKE_FRINGE_PIXELS, half_width + STROKE_FRINGE_PIXELS);
}

fn stroked(vertex: u32, segment: Segment, stroke: Stroke) -> Varyings {
    let direction = segment_direction(segment);
    let normal = vec2<f32>(-direction.y, direction.x);
    let corner = quad_corner(vertex);
    let at_end = corner.x > 0.0;
    let reach = stroke_reach(stroke);
    let along = corner.x * reach.x;
    let across = corner.y * reach.y;
    var clip = select(segment.start_clip, segment.end_clip, at_end);
    clip = vec4<f32>(clip.xy + pixels_to_ndc(direction * along + normal * across) * clip.w, clip.zw);

    var out = empty_varyings();
    out.position = finish(clip, stroke.depth_bias, stroke.in_front);
    out.depth = select(view_depth(segment.start), view_depth(segment.end), at_end);
    out.relative = select(segment.start, segment.end, at_end);
    if finishes_strokes() {
        let span = length(segment.along_pixels);
        let from_start = along + select(0.0, span, at_end);
        out.stroke = vec3<f32>(from_start, across, 1.0) * clip.w;
        out.stroke_extent = vec3<f32>(span, stroke.width * pixels_per_point() * 0.5, select(0.0, 1.0, stroke.capped));
    }
    return out;
}

fn stroke_coverage(in: Varyings) -> f32 {
    if in.stroke.z <= 0.0 {
        return 1.0;
    }
    let along = in.stroke.x / in.stroke.z;
    let across = abs(in.stroke.y / in.stroke.z);
    let span = in.stroke_extent.x;
    let half_width = in.stroke_extent.y;
    let beyond = max(-along, along - span);
    let across_coverage = clamp(half_width + 0.5 - across, 0.0, 1.0);
    if in.stroke_extent.z > 0.5 {
        let from_end = length(vec2<f32>(max(beyond, 0.0), across));
        return clamp(half_width + 0.5 - from_end, 0.0, 1.0);
    }
    return min(across_coverage, clamp(0.5 - beyond, 0.0, 1.0));
}

@vertex
fn vs_line(@builtin(vertex_index) vertex: u32, line: LineInstance) -> Varyings {
    return line_varyings(vertex, line, line.along >= 0.0 && (line.flags & SOLID_WHERE_SEEN) == 0u);
}

@vertex
fn vs_hidden_line(@builtin(vertex_index) vertex: u32, line: LineInstance) -> Varyings {
    return line_varyings(vertex, line, line.along >= 0.0);
}

fn line_varyings(vertex: u32, line: LineInstance, dashed: bool) -> Varyings {
    let line_start = from_anchor(line.start);
    let line_end = from_anchor(line.end);
    if is_before_near(line_start, line_end) {
        return empty_varyings();
    }
    let segment = clipped_segment(line_start, line_end);

    let stroke = Stroke(line.width, line.depth_bias, line.flags & IN_FRONT, line.color.a >= OPAQUE_ALPHA);
    var out = stroked(vertex, segment, stroke);
    out.color = line.color;
    out.pick = line.pick;
    out.sectioned = line.flags & SECTIONED;
    if dashed {
        let corner = quad_corner(vertex);
        let at_end = corner.x > 0.0;
        let clipped_length = distance(segment.start, segment.end);
        let points_per_unit = length(segment.along_pixels) / (max(clipped_length, 1e-12) * pixels_per_point());
        let along_start = line.along + distance(line_start, segment.start);
        let beyond_points = corner.x * stroke_reach(stroke).x / pixels_per_point();
        out.dash_points = max((along_start + select(0.0, clipped_length, at_end)) * points_per_unit + beyond_points, 0.0);
    }
    return out;
}

struct MarkerInstance {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) diameter: f32,
    @location(3) pick: u32,
    @location(4) depth_bias: f32,
    @location(5) flags: u32,
}

@vertex
fn vs_marker(@builtin(vertex_index) vertex: u32, marker: MarkerInstance) -> Varyings {
    let position = from_anchor(marker.position);
    let depth = view_depth(position);
    if depth < view.forward_near.w * 1.01 {
        return empty_varyings();
    }
    let corner = quad_corner(vertex);
    let diameter = marker.diameter * pixels_per_point();
    let radius = diameter * 0.5 + 1.0;
    let center = to_clip(position);
    let clip = vec4<f32>(center.xy + pixels_to_ndc(corner * radius) * center.w, center.zw);

    var out = empty_varyings();
    out.position = finish(clip, marker.depth_bias, marker.flags & IN_FRONT);
    out.color = marker.color;
    out.pick = marker.pick;
    out.depth = depth;
    out.relative = position;
    out.sectioned = marker.flags & SECTIONED;
    out.local = corner * radius;
    out.diameter = diameter;
    return out;
}

struct FillVertex {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) pick: u32,
    @location(3) depth_bias: f32,
    @location(4) flags: u32,
}

@vertex
fn vs_fill(fill: FillVertex) -> Varyings {
    var out = empty_varyings();
    let position = from_anchor(fill.position);
    out.position = finish(to_clip(position), fill.depth_bias, fill.flags & IN_FRONT);
    out.color = fill.color;
    out.pick = fill.pick;
    out.depth = view_depth(position);
    out.relative = position;
    out.sectioned = fill.flags & SECTIONED;
    return out;
}

struct MeshVertex {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) face: u32,
}

const ABSENT_NORMAL_SUM: f32 = -1.9998;

fn unfolded(normal: vec2<f32>) -> vec3<f32> {
    if normal.x + normal.y < ABSENT_NORMAL_SUM {
        return vec3<f32>(0.0);
    }
    let z = 1.0 - abs(normal.x) - abs(normal.y);
    let side = select(vec2<f32>(-1.0), vec2<f32>(1.0), normal >= vec2<f32>(0.0));
    let xy = select(normal, (1.0 - abs(normal.yx)) * side, z < 0.0);
    return normalize(vec3<f32>(xy, z));
}

fn unpack_color(packed: u32) -> vec4<f32> {
    let channels = vec4<u32>(packed, packed >> 8u, packed >> 16u, packed >> 24u) & vec4<u32>(255u);
    return vec4<f32>(channels) / 255.0;
}

fn turned(vector: vec3<f32>) -> vec3<f32> {
    return mesh.turn_x.xyz * vector.x + mesh.turn_y.xyz * vector.y + mesh.turn_z.xyz * vector.z;
}

@vertex
fn vs_mesh(vertex: MeshVertex) -> Varyings {
    let relative = turned(vertex.position) + from_anchor(mesh.offset.xyz);
    let face = min(vertex.face, max(mesh.faces_columns.x, 1u) - 1u);
    let columns = max(mesh.faces_columns.y, 1u);
    let style = textureLoad(face_styles, vec2<u32>(face % columns, face / columns), 0);

    var out = empty_varyings();
    out.position = finish(to_clip(relative), 1.0, BEHIND);
    out.color = unpack_color(style.x);
    out.pick = style.y;
    out.depth = view_depth(relative);
    out.relative = relative;
    out.normal = turned(unfolded(vertex.normal));
    out.sectioned = SECTIONED | select(0u, CAPPABLE, mesh.faces_columns.z != 0u);
    return out;
}

struct SilhouetteStyle {
    offset_width: vec4<f32>,
    color: vec4<f32>,
    turn_x_bias: vec4<f32>,
    turn_y_dashed: vec4<f32>,
    turn_z: vec4<f32>,
}

@group(1) @binding(3) var<uniform> silhouette: SilhouetteStyle;

struct SilhouetteTriangle {
    @location(0) first: vec3<f32>,
    @location(1) second: vec3<f32>,
    @location(2) third: vec3<f32>,
    @location(3) first_normal: vec2<f32>,
    @location(4) second_normal: vec2<f32>,
    @location(5) third_normal: vec2<f32>,
}

fn silhouette_turned(vector: vec3<f32>) -> vec3<f32> {
    return silhouette.turn_x_bias.xyz * vector.x + silhouette.turn_y_dashed.xyz * vector.y + silhouette.turn_z.xyz * vector.z;
}

fn silhouette_placed(position: vec3<f32>) -> vec3<f32> {
    return silhouette_turned(position) + from_anchor(silhouette.offset_width.xyz);
}

fn facing(position: vec3<f32>, normal: vec2<f32>) -> f32 {
    return dot(silhouette_turned(unfolded(normal)), toward_eye(position));
}

fn crossing(head: vec3<f32>, tail: vec3<f32>, head_facing: f32, tail_facing: f32) -> vec3<f32> {
    return mix(head, tail, head_facing / (head_facing - tail_facing));
}

fn screen_dash_points(segment: Segment, vertex: u32) -> f32 {
    let clip = select(segment.start_clip, segment.end_clip, quad_corner(vertex).x > 0.0);
    let pixels = ndc_to_pixels(clip);
    let direction = segment_direction(segment);
    let along = select(pixels.y, pixels.x, abs(direction.x) >= abs(direction.y));
    return along / pixels_per_point() + 1e4;
}

@vertex
fn vs_silhouette(@builtin(vertex_index) vertex: u32, triangle: SilhouetteTriangle) -> Varyings {
    return silhouette_stroke(vertex, triangle, silhouette.turn_y_dashed.w > 0.5);
}

@vertex
fn vs_hidden_silhouette(@builtin(vertex_index) vertex: u32, triangle: SilhouetteTriangle) -> Varyings {
    return silhouette_stroke(vertex, triangle, true);
}

fn silhouette_stroke(vertex: u32, triangle: SilhouetteTriangle, dashed: bool) -> Varyings {
    let first = silhouette_placed(triangle.first);
    let second = silhouette_placed(triangle.second);
    let third = silhouette_placed(triangle.third);
    let first_facing = facing(first, triangle.first_normal);
    let second_facing = facing(second, triangle.second_normal);
    let third_facing = facing(third, triangle.third_normal);
    let front = vec3<bool>(first_facing >= 0.0, second_facing >= 0.0, third_facing >= 0.0);
    if all(front) || !any(front) {
        return empty_varyings();
    }

    var lone = first;
    var lone_facing = first_facing;
    var one = second;
    var one_facing = second_facing;
    var other = third;
    var other_facing = third_facing;
    if front.y != front.x && front.y != front.z {
        lone = second;
        lone_facing = second_facing;
        one = first;
        one_facing = first_facing;
    } else if front.z != front.x && front.z != front.y {
        lone = third;
        lone_facing = third_facing;
        other = first;
        other_facing = first_facing;
    }
    let start = crossing(lone, one, lone_facing, one_facing);
    let end = crossing(lone, other, lone_facing, other_facing);
    if is_before_near(start, end) {
        return empty_varyings();
    }
    let segment = clipped_segment(start, end);

    let stroke = Stroke(silhouette.offset_width.w, silhouette.turn_x_bias.w, BEHIND, silhouette.color.a >= OPAQUE_ALPHA);
    var out = stroked(vertex, segment, stroke);
    out.color = silhouette.color;
    out.sectioned = SECTIONED;
    if dashed {
        out.dash_points = screen_dash_points(segment, vertex);
    }
    return out;
}

@vertex
fn vs_grid(@builtin(vertex_index) vertex: u32) -> Varyings {
    let local = quad_corner(vertex) * grid.origin_extent.w;
    let position = grid.origin_extent.xyz
        + grid.axis_u_spacing.xyz * local.x
        + grid.axis_v_fade.xyz * local.y;

    var out = empty_varyings();
    out.position = finish(to_clip(position), GRID_DEPTH_BIAS, BEHIND);
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
fn fs_fill(in: Varyings) -> @location(0) vec4<f32> {
    if is_cut(in) {
        discard;
    }
    return in.color;
}

@fragment
fn fs_line(in: Varyings) -> @location(0) vec4<f32> {
    if in.color.a <= 0.0 || is_cut(in) {
        discard;
    }
    if in.dash_points >= 0.0 && fract(in.dash_points / DASH_PERIOD_POINTS) > DASH_DRAWN_FRACTION {
        discard;
    }
    let coverage = stroke_coverage(in);
    if coverage <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}

const AMBIENT: f32 = 0.07;
const KEY_LIGHT: f32 = 0.76;
const HEADLIGHT: f32 = 0.28;
const SPECULAR: f32 = 0.15;
const SHININESS: f32 = 40.0;

const GROUND_AMBIENT: f32 = 0.02;
const SKY_AMBIENT: f32 = 0.093;
const ENHANCED_KEY_LIGHT: f32 = 0.77;
const ENHANCED_FILL_LIGHT: f32 = 0.26;
const ENHANCED_HEADLIGHT: f32 = 0.185;
const ENHANCED_SPECULAR: f32 = 0.4;
const ENHANCED_SHININESS: f32 = 72.0;
const SHEEN: f32 = 0.08;
const SHEEN_SHININESS: f32 = 8.0;
const RIM: f32 = 0.05;
const RIM_EXPONENT: f32 = 3.0;
const RIM_WHITENING: f32 = 0.5;
const REFLECTANCE_PER_LUMINANCE: f32 = 1.6;
const LUMINANCE: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

fn uses_enhanced_shading() -> bool {
    return view.light.w > 0.5;
}

fn facing_normal(in: Varyings, eye: vec3<f32>) -> vec3<f32> {
    var normal = in.normal;
    if dot(normal, normal) < 1e-12 {
        normal = eye;
    }
    normal = normalize(normal);
    if dot(normal, eye) < 0.0 {
        normal = -normal;
    }
    return normal;
}

fn standard_shade(color: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> vec3<f32> {
    let toward_light = view.light.xyz;
    let key = max(dot(normal, toward_light), 0.0);
    let head = max(dot(normal, eye), 0.0);
    let halfway = normalize(toward_light + eye);
    let shine = pow(max(dot(normal, halfway), 0.0), SHININESS) * SPECULAR;
    let shade = AMBIENT + KEY_LIGHT * key + HEADLIGHT * head;
    return color * shade + vec3<f32>(shine);
}

fn enhanced_shade(color: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> vec3<f32> {
    let key_light = view.light.xyz;
    let hemisphere = mix(GROUND_AMBIENT, SKY_AMBIENT, normal.z * 0.5 + 0.5);
    let key = max(dot(normal, key_light), 0.0);
    let fill = max(dot(normal, view.fill_light.xyz), 0.0);
    let facing = clamp(dot(normal, eye), 0.0, 1.0);
    let diffuse = hemisphere
        + ENHANCED_KEY_LIGHT * key
        + ENHANCED_FILL_LIGHT * fill
        + ENHANCED_HEADLIGHT * facing;

    let lightness = to_srgb(vec3<f32>(dot(color, LUMINANCE))).x;
    let reflectance = clamp(lightness * REFLECTANCE_PER_LUMINANCE, 0.0, 1.0);
    let alignment = max(dot(normal, normalize(key_light + eye)), 0.0);
    let highlight = ENHANCED_SPECULAR * pow(alignment, ENHANCED_SHININESS)
        + SHEEN * pow(alignment, SHEEN_SHININESS);
    let rim = RIM * pow(1.0 - facing, RIM_EXPONENT);
    let rim_color = mix(color, vec3<f32>(1.0), RIM_WHITENING);
    return color * diffuse + (vec3<f32>(highlight) + rim_color * rim) * reflectance;
}

fn shade(in: Varyings) -> vec4<f32> {
    let eye = toward_eye(in.relative);
    let normal = facing_normal(in, eye);
    return lit(in.color, normal, eye);
}

fn lit(color: vec4<f32>, normal: vec3<f32>, eye: vec3<f32>) -> vec4<f32> {
    let linear = to_linear(color.rgb);
    if uses_enhanced_shading() {
        return vec4<f32>(to_srgb(enhanced_shade(linear, normal, eye)), color.a);
    }
    return vec4<f32>(to_srgb(standard_shade(linear, normal, eye)), color.a);
}

@fragment
fn fs_mesh(in: Varyings) -> @location(0) vec4<f32> {
    if in.color.a <= 0.0 || is_cut(in) {
        discard;
    }
    return shade(in);
}

fn ray_reach(relative: vec3<f32>) -> vec3<f32> {
    if is_orthographic() {
        return view.forward_near.xyz * view_depth(relative);
    }
    return relative;
}

struct Cap {
    found: bool,
    relative: vec3<f32>,
    plane: u32,
}

fn cap_behind(relative: vec3<f32>) -> Cap {
    let reach = ray_reach(relative);
    var entry = 0.0;
    var plane = MAX_SECTION_PLANES;
    for (var index = 0u; index < section_count(); index += 1u) {
        let section = view.section_planes[index];
        let along = dot(section.xyz, reach);
        if along < 0.0 {
            let at = 1.0 - beyond(section, relative) / along;
            if at > entry {
                entry = at;
                plane = index;
            }
        }
    }
    let found = plane < MAX_SECTION_PLANES && entry < 1.0;
    return Cap(found, relative - reach * (1.0 - min(entry, 1.0)), min(plane, MAX_SECTION_PLANES - 1u));
}

fn is_back_face(in: Varyings) -> bool {
    let flat_normal = cross(dpdx(in.relative), dpdy(in.relative));
    let outward = select(-flat_normal, flat_normal, dot(flat_normal, in.normal) >= 0.0);
    return dot(outward, toward_eye(in.relative)) < 0.0;
}

fn sectioned_face_depth(in: Varyings) -> f32 {
    let slope = max(abs(dpdx(in.position.z)), abs(dpdy(in.position.z)));
    return max(in.position.z - FACE_SLOPE_BIAS * slope, 0.0);
}

fn cap_depth(relative: vec3<f32>) -> f32 {
    let clip = to_clip(relative);
    return clamp(layered_depth(clip, CAP_DEPTH_BIAS, BEHIND) / clip.w, 0.0, 1.0);
}

struct Sectioned {
    cut: bool,
    capped: bool,
    cap: Cap,
    depth: f32,
    hatch: f32,
}

fn sectioned(in: Varyings) -> Sectioned {
    let back = is_back_face(in);
    let face_depth = sectioned_face_depth(in);
    let cap = cap_behind(in.relative);
    let hatch = view.section_hatches[cap.plane];
    let across = dot(cap.relative, hatch.xyz) + hatch.w;
    let width = max(fwidth(across), 1e-6);
    let distance = abs(fract(across + 0.5) - 0.5) / width;
    let line = clamp(HATCH_WIDTH_POINTS * pixels_per_point() * 0.5 + 0.5 - distance, 0.0, 1.0);
    let hatched = select(0.0, line, dot(hatch.xyz, hatch.xyz) > 0.0);
    let capped = back && cap.found && (in.sectioned & CAPPABLE) != 0u;
    return Sectioned(is_cut_away(in.relative), capped, cap, select(face_depth, cap_depth(in.relative), capped), hatched);
}

fn hatched(color: vec3<f32>, section: Sectioned) -> vec4<f32> {
    return vec4<f32>(mix(color, color * HATCH_SHADE, section.hatch), 1.0);
}

fn cap_color(in: Varyings, section: Sectioned) -> vec4<f32> {
    let eye = toward_eye(section.cap.relative);
    var normal = view.section_planes[section.cap.plane].xyz;
    if dot(normal, eye) < 0.0 {
        normal = -normal;
    }
    let filled = lit(vec4<f32>(in.color.rgb * CAP_SHADE, 1.0), normal, eye);
    return hatched(filled.rgb, section);
}

struct SectionedOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fs_mesh_sectioned(in: Varyings) -> SectionedOutput {
    let section = sectioned(in);
    if in.color.a <= 0.0 || section.cut {
        discard;
    }
    if section.capped {
        return SectionedOutput(cap_color(in, section), section.depth);
    }
    return SectionedOutput(shade(in), section.depth);
}

@fragment
fn fs_color_sectioned(in: Varyings) -> SectionedOutput {
    let section = sectioned(in);
    if section.cut {
        discard;
    }
    if section.capped {
        return SectionedOutput(hatched(in.color.rgb * CAP_SHADE, section), section.depth);
    }
    return SectionedOutput(in.color, section.depth);
}

const TAU: f32 = 6.2831853;
const ZEBRA_LIGHT: f32 = 1.3;
const ZEBRA_DARK: f32 = 0.08;
const CHROME_TINT: f32 = 0.5;
const SKY_LOW: vec3<f32> = vec3<f32>(0.55, 0.62, 0.72);
const SKY_HIGH: vec3<f32> = vec3<f32>(0.95, 0.97, 1.0);
const GROUND_NEAR: vec3<f32> = vec3<f32>(0.46, 0.43, 0.4);
const GROUND_FAR: vec3<f32> = vec3<f32>(0.26, 0.26, 0.28);
const HORIZON_WIDTH: f32 = 0.015;
const PANEL_DIRECTION: vec3<f32> = vec3<f32>(0.48, -0.56, 0.68);
const PANEL_SHARPNESS: f32 = 90.0;
const PANEL_BRIGHTNESS: f32 = 0.9;

fn is_zebra() -> bool {
    return view.reflection_along.w > 0.5;
}

fn zebra(color: vec3<f32>, reflected: vec3<f32>) -> vec3<f32> {
    let across = view.reflection_across.xyz;
    let along = view.reflection_along.xyz;
    let stripes = view.reflection_across.w;
    let x = dot(reflected, across);
    let y = dot(reflected, along);
    let turns = atan2(y, x) / TAU * stripes;
    let opposite = atan2(-y, -x) / TAU * stripes;
    let width = max(min(fwidth(turns), fwidth(opposite)), 1e-4);
    let distance = abs(fract(turns) - 0.5);
    let light = smoothstep(0.25 - width, 0.25 + width, distance);
    let bright = min(color * ZEBRA_LIGHT, vec3<f32>(1.0));
    return mix(color * ZEBRA_DARK, bright, light);
}

fn environment(reflected: vec3<f32>) -> vec3<f32> {
    let up = reflected.z;
    let sky = mix(SKY_LOW, SKY_HIGH, smoothstep(0.0, 0.7, up));
    let ground = mix(GROUND_NEAR, GROUND_FAR, smoothstep(0.0, -0.7, up));
    let horizon = smoothstep(-HORIZON_WIDTH, HORIZON_WIDTH, up);
    let panel = pow(max(dot(reflected, normalize(PANEL_DIRECTION)), 0.0), PANEL_SHARPNESS);
    return mix(ground, sky, horizon) + vec3<f32>(panel * PANEL_BRIGHTNESS);
}

fn chrome(color: vec3<f32>, reflected: vec3<f32>) -> vec3<f32> {
    let brightest = max(max(color.r, color.g), max(color.b, 1e-3));
    let tint = mix(vec3<f32>(1.0), color / brightest, CHROME_TINT);
    return min(environment(reflected) * tint, vec3<f32>(1.0));
}

fn reflective(in: Varyings) -> vec4<f32> {
    let eye = toward_eye(in.relative);
    let normal = facing_normal(in, eye);
    let reflected = reflect(-eye, normal);
    if is_zebra() {
        return vec4<f32>(zebra(in.color.rgb, reflected), in.color.a);
    }
    return vec4<f32>(chrome(in.color.rgb, reflected), in.color.a);
}

@fragment
fn fs_reflective(in: Varyings) -> @location(0) vec4<f32> {
    return reflective(in);
}

@fragment
fn fs_reflective_sectioned(in: Varyings) -> SectionedOutput {
    let section = sectioned(in);
    let color = reflective(in);
    if section.cut {
        discard;
    }
    if section.capped {
        return SectionedOutput(cap_color(in, section), section.depth);
    }
    return SectionedOutput(color, section.depth);
}

fn marker_coverage(in: Varyings) -> f32 {
    return clamp(in.diameter * 0.5 - length(in.local) + 0.5, 0.0, 1.0);
}

@fragment
fn fs_marker(in: Varyings) -> @location(0) vec4<f32> {
    let coverage = marker_coverage(in);
    if coverage <= 0.0 || in.color.a <= 0.0 || is_cut(in) {
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
    if in.pick == 0u || is_cut(in) {
        discard;
    }
    return PickOutput(in.pick, bitcast<u32>(in.depth));
}

@fragment
fn fs_mesh_pick(in: Varyings) -> PickOutput {
    return PickOutput(in.pick, bitcast<u32>(in.depth));
}

struct SectionedPick {
    @location(0) id: u32,
    @location(1) depth: u32,
    @builtin(frag_depth) frag_depth: f32,
}

@fragment
fn fs_mesh_pick_sectioned(in: Varyings) -> SectionedPick {
    let section = sectioned(in);
    if section.cut {
        discard;
    }
    if section.capped {
        return SectionedPick(0u, bitcast<u32>(view_depth(section.cap.relative)), section.depth);
    }
    return SectionedPick(in.pick, bitcast<u32>(in.depth), section.depth);
}

@fragment
fn fs_marker_pick(in: Varyings) -> PickOutput {
    if in.pick == 0u || marker_coverage(in) <= 0.0 || is_cut(in) {
        discard;
    }
    return PickOutput(in.pick, bitcast<u32>(in.depth));
}
