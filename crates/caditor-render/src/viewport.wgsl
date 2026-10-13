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
    sun: vec4<f32>,
    sun_colour: vec4<f32>,
    exposure_shadow: vec4<f32>,
    bounce: vec4<f32>,
    ground: vec4<f32>,
    sky_view: vec4<f32>,
    shadow_matrix: mat4x4<f32>,
    relative_from_light: mat4x4<f32>,
    occlusion: vec4<f32>,
    occlusion_frame: vec4<f32>,
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
}

struct Grid {
    origin_extent: vec4<f32>,
    axis_u_spacing: vec4<f32>,
    axis_v_fade: vec4<f32>,
    color: vec4<f32>,
}

struct Picture {
    corners: array<vec4<f32>, 4>,
    look: vec4<f32>,
    pick: vec4<u32>,
}

struct PictureVaryings {
    @builtin(position) position: vec4<f32>,
    @location(0) texel: vec2<f32>,
    @location(1) depth: f32,
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
@group(1) @binding(4) var styles: texture_2d<u32>;
@group(1) @binding(5) var picture_texture: texture_2d<f32>;
@group(1) @binding(6) var picture_sampler: sampler;
@group(1) @binding(7) var<uniform> picture: Picture;
@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;
@group(2) @binding(2) var lit_albedo: texture_2d<f32>;
@group(2) @binding(3) var lit_normals: texture_2d<f32>;
@group(2) @binding(4) var lit_depth: texture_2d<u32>;
@group(3) @binding(0) var occlusion_map: texture_2d<f32>;
@group(3) @binding(1) var occlusion_surfaces: texture_2d<u32>;
@group(1) @binding(8) var surface_texels: texture_2d<u32>;
@group(1) @binding(9) var raw_occlusion: texture_2d<f32>;

const CULLED: vec4<f32> = vec4<f32>(0.0, 0.0, 2.0, 1.0);
const ORTHOGRAPHIC_DEPTH_BIAS: f32 = 0.03;
const HALF_DEPTH_RANGE: f32 = 0.5;
const GRID_DEPTH_BIAS: f32 = 0.99998;
const REFERENCE_FILL_DEPTH_BIAS: f32 = 0.99998;
const MODEL_FILL_DEPTH_BIAS: f32 = 1.00002;
const FRONT_FILL_DEPTH_BIAS: f32 = 1.0;
const REFERENCE_LINE_DEPTH_BIAS: f32 = 1.00001;
const MODEL_LINE_DEPTH_BIAS: f32 = 1.00003;
const FRONT_LINE_DEPTH_BIAS: f32 = 1.002;
const REFERENCE_MARKER_DEPTH_BIAS: f32 = 1.00002;
const MODEL_MARKER_DEPTH_BIAS: f32 = 1.00004;
const FRONT_MARKER_DEPTH_BIAS: f32 = 1.004;
const BEHIND: u32 = 0u;
const IN_FRONT: u32 = 1u;
const SECTIONED: u32 = 2u;
const CAPPABLE: u32 = 4u;
const SOLID_WHERE_SEEN: u32 = 8u;
const FINISH_SHIFT: u32 = 8u;
const FINISH_PRESENT: u32 = 0x10000u;
const FINISH_LEVEL: u32 = 255u;
const DRAWN: u32 = 0x80000000u;
const LINKED: u32 = 0x40000000u;
const STYLE_INDEX: u32 = 0x3fffffffu;
const START_ROUND: u32 = 1u;
const END_ROUND: u32 = 2u;
const START_JOINED: u32 = 4u;
const END_JOINED: u32 = 8u;
const MIN_JOINED_PIXELS: f32 = 1e-3;
const SEAM_OFFSET_PIXELS: f32 = 0.0073;
const MAX_SECTION_PLANES: u32 = 6u;
const FACE_SLOPE_BIAS: f32 = 2.0;
const CAP_DEPTH_BIAS: f32 = 1.0002;
const CAP_SHADE: f32 = 0.7;
const HATCH_SHADE: f32 = 0.25;
const HATCH_WIDTH_POINTS: f32 = 1.0;
const DASH_PERIOD_POINTS: f32 = 10.0;
const DASH_DRAWN_FRACTION: f32 = 0.6;
const DASH_LEAD_POINTS: f32 = DASH_PERIOD_POINTS * 64.0;
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
    @location(11) @interpolate(flat) joints: vec4<f32>,
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

fn layer_depth_bias(flags: u32, reference: f32, model: f32, front: f32) -> f32 {
    if (flags & IN_FRONT) != BEHIND {
        return front;
    }
    return select(reference, model, (flags & SECTIONED) != 0u);
}

fn line_depth_bias(flags: u32) -> f32 {
    return layer_depth_bias(flags, REFERENCE_LINE_DEPTH_BIAS, MODEL_LINE_DEPTH_BIAS, FRONT_LINE_DEPTH_BIAS);
}

fn marker_depth_bias(flags: u32) -> f32 {
    return layer_depth_bias(flags, REFERENCE_MARKER_DEPTH_BIAS, MODEL_MARKER_DEPTH_BIAS, FRONT_MARKER_DEPTH_BIAS);
}

fn fill_depth_bias(flags: u32) -> f32 {
    return layer_depth_bias(flags, REFERENCE_FILL_DEPTH_BIAS, MODEL_FILL_DEPTH_BIAS, FRONT_FILL_DEPTH_BIAS);
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
    out.joints = vec4<f32>(0.0);
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
    @location(0) previous: vec3<f32>,
    @location(1) previous_word: u32,
    @location(2) start: vec3<f32>,
    @location(3) along: f32,
    @location(4) pick: u32,
    @location(5) word: u32,
    @location(6) end: vec3<f32>,
    @location(7) end_word: u32,
    @location(8) next: vec3<f32>,
}

struct LineStyle {
    color: vec4<f32>,
    width: f32,
    flags: u32,
}

fn style_texel(index: u32) -> vec4<u32> {
    let columns = max(textureDimensions(styles).x, 1u);
    return textureLoad(styles, vec2<u32>(index % columns, index / columns), 0);
}

fn line_style(word: u32) -> LineStyle {
    let texel = style_texel(word & STYLE_INDEX);
    return LineStyle(unpack_color(texel.x), bitcast<f32>(texel.y), texel.z);
}

struct Joins {
    ends: u32,
    previous: vec2<f32>,
    next: vec2<f32>,
}

fn unjoined() -> Joins {
    return Joins(0u, vec2<f32>(0.0), vec2<f32>(0.0));
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

fn is_beyond_near(position: vec3<f32>) -> bool {
    return view_depth(position) >= near_limit();
}

fn screen_pixels(position: vec3<f32>) -> vec2<f32> {
    return ndc_to_pixels(to_clip(position));
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

fn end_reach(stroke: Stroke, joins: Joins, at_end: bool) -> f32 {
    let reach = stroke_reach(stroke);
    let joined = (joins.ends & select(START_JOINED, END_JOINED, at_end)) != 0u;
    return select(reach.x, reach.y, joined);
}

fn stroke_ends(stroke: Stroke, joins: Joins) -> u32 {
    let capped = select(0u, START_ROUND | END_ROUND, stroke.capped);
    let joined = joins.ends & (START_JOINED | END_JOINED);
    let rounded_by_joins = select(0u, START_ROUND, (joined & START_JOINED) != 0u) | select(0u, END_ROUND, (joined & END_JOINED) != 0u);
    return capped | joined | rounded_by_joins;
}

fn in_segment_frame(vector: vec2<f32>, direction: vec2<f32>, normal: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(dot(vector, direction), dot(vector, normal));
}

fn stroked(vertex: u32, segment: Segment, stroke: Stroke, joins: Joins) -> Varyings {
    let direction = segment_direction(segment);
    let normal = vec2<f32>(-direction.y, direction.x);
    let corner = quad_corner(vertex);
    let at_end = corner.x > 0.0;
    let reach = stroke_reach(stroke);
    let along = corner.x * end_reach(stroke, joins, at_end);
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
        out.stroke_extent = vec3<f32>(span, stroke.width * pixels_per_point() * 0.5, f32(stroke_ends(stroke, joins)));
        out.joints = vec4<f32>(in_segment_frame(joins.previous, direction, normal), in_segment_frame(joins.next, direction, normal));
    }
    return out;
}

fn neighbour_owns(offset: vec2<f32>, away: vec2<f32>, neighbour: vec2<f32>, half_width: f32, outgoing: bool) -> bool {
    let span = length(neighbour);
    if span <= 0.0 {
        return false;
    }
    let toward = neighbour / span;
    let side = dot(offset, away - toward);
    let on_neighbour_side = side < select(SEAM_OFFSET_PIXELS, -SEAM_OFFSET_PIXELS, outgoing);
    let from_joint = dot(offset, toward);
    if !on_neighbour_side || from_joint > span {
        return false;
    }
    return length(offset - toward * max(from_joint, 0.0)) < half_width + 0.5;
}

fn stroke_coverage(in: Varyings) -> f32 {
    if in.stroke.z <= 0.0 {
        return 1.0;
    }
    let point = in.stroke.xy / in.stroke.z;
    let along = point.x;
    let across = abs(point.y);
    let span = in.stroke_extent.x;
    let half_width = in.stroke_extent.y;
    let ends = u32(in.stroke_extent.z + 0.5);
    if (ends & START_JOINED) != 0u && neighbour_owns(point, vec2<f32>(1.0, 0.0), in.joints.xy, half_width, true) {
        return 0.0;
    }
    if (ends & END_JOINED) != 0u && neighbour_owns(point - vec2<f32>(span, 0.0), vec2<f32>(-1.0, 0.0), in.joints.zw, half_width, false) {
        return 0.0;
    }
    let past_end = along - span > -along;
    let beyond = max(-along, along - span);
    let across_coverage = clamp(half_width + 0.5 - across, 0.0, 1.0);
    if (ends & select(START_ROUND, END_ROUND, past_end)) != 0u {
        let from_end = length(vec2<f32>(max(beyond, 0.0), across));
        return clamp(half_width + 0.5 - from_end, 0.0, 1.0);
    }
    return min(across_coverage, clamp(0.5 - beyond, 0.0, 1.0));
}

@vertex
fn vs_line(@builtin(vertex_index) vertex: u32, line: LineInstance) -> Varyings {
    let style = line_style(line.word);
    return line_varyings(vertex, line, style, line.along >= 0.0 && (style.flags & SOLID_WHERE_SEEN) == 0u);
}

@vertex
fn vs_hidden_line(@builtin(vertex_index) vertex: u32, line: LineInstance) -> Varyings {
    return line_varyings(vertex, line, line_style(line.word), line.along >= 0.0);
}

fn joined_neighbour(linked: bool, far: vec3<f32>, joint: vec2<f32>) -> vec2<f32> {
    if !linked || !is_beyond_near(far) {
        return vec2<f32>(0.0);
    }
    let toward = screen_pixels(far) - joint;
    return select(vec2<f32>(0.0), toward, length(toward) > MIN_JOINED_PIXELS);
}

fn line_joins(line: LineInstance, start: vec3<f32>, end: vec3<f32>) -> Joins {
    var joins = unjoined();
    if !finishes_strokes() || !is_beyond_near(start) || !is_beyond_near(end) {
        return joins;
    }
    let start_pixels = screen_pixels(start);
    let end_pixels = screen_pixels(end);
    if length(end_pixels - start_pixels) <= MIN_JOINED_PIXELS {
        return joins;
    }
    joins.previous = joined_neighbour((line.previous_word & LINKED) != 0u, from_anchor(line.previous), start_pixels);
    joins.next = joined_neighbour((line.end_word & LINKED) != 0u, from_anchor(line.next), end_pixels);
    joins.ends = select(0u, START_JOINED, any(joins.previous != vec2<f32>(0.0))) | select(0u, END_JOINED, any(joins.next != vec2<f32>(0.0)));
    return joins;
}

fn line_varyings(vertex: u32, line: LineInstance, style: LineStyle, dashed: bool) -> Varyings {
    if (line.word & DRAWN) == 0u {
        return empty_varyings();
    }
    let line_start = from_anchor(line.start);
    let line_end = from_anchor(line.end);
    if is_before_near(line_start, line_end) {
        return empty_varyings();
    }
    let segment = clipped_segment(line_start, line_end);

    let stroke = Stroke(style.width, line_depth_bias(style.flags), style.flags & IN_FRONT, style.color.a >= OPAQUE_ALPHA);
    let joins = line_joins(line, line_start, line_end);
    var out = stroked(vertex, segment, stroke, joins);
    out.color = style.color;
    out.pick = line.pick;
    out.sectioned = style.flags & SECTIONED;
    if dashed {
        let corner = quad_corner(vertex);
        let at_end = corner.x > 0.0;
        let clipped_length = distance(segment.start, segment.end);
        let points_per_unit = length(segment.along_pixels) / (max(clipped_length, 1e-12) * pixels_per_point());
        let along_start = line.along + distance(line_start, segment.start);
        let beyond_points = corner.x * end_reach(stroke, joins, at_end) / pixels_per_point();
        out.dash_points = max((along_start + select(0.0, clipped_length, at_end)) * points_per_unit + beyond_points + DASH_LEAD_POINTS, 0.0);
    }
    return out;
}

struct MarkerInstance {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) diameter: f32,
    @location(3) pick: u32,
    @location(4) flags: u32,
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
    out.position = finish(clip, marker_depth_bias(marker.flags), marker.flags & IN_FRONT);
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
    @location(1) style: u32,
}

@vertex
fn vs_fill(fill: FillVertex) -> Varyings {
    let style = style_texel(fill.style);
    let flags = style.z;
    var out = empty_varyings();
    let position = from_anchor(fill.position);
    out.position = finish(to_clip(position), fill_depth_bias(flags), flags & IN_FRONT);
    out.color = unpack_color(style.x);
    out.pick = style.y;
    out.depth = view_depth(position);
    out.relative = position;
    out.sectioned = flags & SECTIONED;
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
    out.sectioned = SECTIONED | select(0u, CAPPABLE, mesh.faces_columns.z != 0u) | (style.z << FINISH_SHIFT);
    return out;
}

@vertex
fn vs_shadow(vertex: MeshVertex) -> Varyings {
    let relative = turned(vertex.position) + from_anchor(mesh.offset.xyz);
    let face = min(vertex.face, max(mesh.faces_columns.x, 1u) - 1u);
    let columns = max(mesh.faces_columns.y, 1u);
    let style = textureLoad(face_styles, vec2<u32>(face % columns, face / columns), 0);

    var out = empty_varyings();
    out.position = view.shadow_matrix * vec4<f32>(relative, 1.0);
    out.color = unpack_color(style.x);
    if out.color.a <= 0.0 {
        out.position = CULLED;
    }
    out.relative = relative;
    out.normal = turned(unfolded(vertex.normal));
    out.sectioned = SECTIONED | (style.z << FINISH_SHIFT);
    return out;
}

struct LitTexel {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) depth: u32,
}

@fragment
fn fs_shadow(in: Varyings) -> LitTexel {
    if is_cut(in) {
        discard;
    }
    let toward_light = view.sun.xyz;
    var normal = in.normal;
    if dot(normal, normal) < 1e-12 {
        normal = toward_light;
    }
    normal = normalize(normal);
    if dot(normal, toward_light) < 0.0 {
        normal = -normal;
    }
    let finish = in.sectioned >> FINISH_SHIFT;
    let metalness = select(0.0, finish_metalness(finish), (finish & FINISH_PRESENT) != 0u);
    let albedo = to_srgb(to_linear(in.color.rgb) * (1.0 - metalness));
    let facing = clamp(dot(normal, toward_light), 0.0, 1.0);
    return LitTexel(vec4<f32>(albedo, facing), vec4<f32>(normal * 0.5 + 0.5, 1.0), bitcast<u32>(in.position.z));
}

struct SilhouetteStyle {
    offset_width: vec4<f32>,
    color: vec4<f32>,
    turn_x: vec4<f32>,
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
    return silhouette.turn_x.xyz * vector.x + silhouette.turn_y_dashed.xyz * vector.y + silhouette.turn_z.xyz * vector.z;
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

    let stroke = Stroke(silhouette.offset_width.w, line_depth_bias(SECTIONED), BEHIND, silhouette.color.a >= OPAQUE_ALPHA);
    var out = stroked(vertex, segment, stroke, unjoined());
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

fn standard_diffuse(normal: vec3<f32>, eye: vec3<f32>) -> f32 {
    let key = max(dot(normal, view.light.xyz), 0.0);
    let head = max(dot(normal, eye), 0.0);
    return AMBIENT + KEY_LIGHT * key + HEADLIGHT * head;
}

fn standard_shade(color: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> vec3<f32> {
    let halfway = normalize(view.light.xyz + eye);
    let shine = pow(max(dot(normal, halfway), 0.0), SHININESS) * SPECULAR;
    return color * standard_diffuse(normal, eye) + vec3<f32>(shine);
}

fn enhanced_diffuse(normal: vec3<f32>, eye: vec3<f32>) -> f32 {
    let hemisphere = mix(GROUND_AMBIENT, SKY_AMBIENT, normal.z * 0.5 + 0.5);
    let key = max(dot(normal, view.light.xyz), 0.0);
    let fill = max(dot(normal, view.fill_light.xyz), 0.0);
    let facing = clamp(dot(normal, eye), 0.0, 1.0);
    return hemisphere
        + ENHANCED_KEY_LIGHT * key
        + ENHANCED_FILL_LIGHT * fill
        + ENHANCED_HEADLIGHT * facing;
}

fn enhanced_shade(color: vec3<f32>, normal: vec3<f32>, eye: vec3<f32>) -> vec3<f32> {
    let key_light = view.light.xyz;
    let facing = clamp(dot(normal, eye), 0.0, 1.0);
    let diffuse = enhanced_diffuse(normal, eye);

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
    let finish = in.sectioned >> FINISH_SHIFT;
    if (finish & FINISH_PRESENT) != 0u {
        if uses_rendered_shading() {
            let sun_seen = sunlit(in.relative, normal);
            let bounce = bounced(in.relative, normal, in.position.xy);
            let occluded = occlusion_at(in.position.xy, in.relative);
            return rendered(in.color, normal, eye, finish, Received(sun_seen, bounce, occluded));
        }
        return finished(in.color, normal, eye, finish);
    }
    return lit(in.color, normal, eye);
}

const PI: f32 = 3.14159265;
const DIELECTRIC_REFLECTANCE: f32 = 0.04;
const METAL_DIFFUSE: f32 = 0.3;
const METAL_REFLECTANCE_GAIN: f32 = 1.5;
const MIN_ROUGHNESS: f32 = 0.08;
const HIGHLIGHT_LIMIT: f32 = 6.0;
const ENVIRONMENT_STRENGTH: f32 = 1.0;
const BLURRED_HORIZON_WIDTH: f32 = 0.6;
const BLURRED_PANEL_SHARPNESS: f32 = 5.0;
const BLURRED_PANEL_BRIGHTNESS: f32 = 0.15;

fn finish_metalness(finish: u32) -> f32 {
    return f32(finish & FINISH_LEVEL) / f32(FINISH_LEVEL);
}

fn finish_roughness(finish: u32) -> f32 {
    return max(f32((finish >> 8u) & FINISH_LEVEL) / f32(FINISH_LEVEL), MIN_ROUGHNESS);
}

fn diffuse_light(normal: vec3<f32>, eye: vec3<f32>) -> f32 {
    if uses_enhanced_shading() {
        return enhanced_diffuse(normal, eye);
    }
    return standard_diffuse(normal, eye);
}

fn schlick(reflectance: vec3<f32>, cosine: f32) -> vec3<f32> {
    return reflectance + (vec3<f32>(1.0) - reflectance) * pow(1.0 - clamp(cosine, 0.0, 1.0), 5.0);
}

fn highlight(normal: vec3<f32>, eye: vec3<f32>, toward_light: vec3<f32>, alpha: f32, reflectance: vec3<f32>) -> vec3<f32> {
    let n_dot_l = max(dot(normal, toward_light), 0.0);
    let n_dot_v = max(dot(normal, eye), 1e-4);
    let halfway = normalize(toward_light + eye);
    let n_dot_h = max(dot(normal, halfway), 0.0);
    let squared = alpha * alpha;
    let spread = n_dot_h * n_dot_h * (squared - 1.0) + 1.0;
    let distribution = squared / (PI * spread * spread);
    let k = alpha * 0.5;
    let visibility = 0.25 / ((n_dot_v * (1.0 - k) + k) * (n_dot_l * (1.0 - k) + k));
    let fresnel = schlick(reflectance, dot(eye, halfway));
    return fresnel * min(distribution * visibility, HIGHLIGHT_LIMIT) * n_dot_l * PI;
}

fn studio(reflected: vec3<f32>, roughness: f32) -> vec3<f32> {
    let up = reflected.z;
    let width = mix(HORIZON_WIDTH, BLURRED_HORIZON_WIDTH, roughness);
    let sky = mix(SKY_LOW, SKY_HIGH, smoothstep(-width, 0.7 + width, up));
    let ground = mix(GROUND_NEAR, GROUND_FAR, smoothstep(width, -0.7 - width, up));
    let horizon = smoothstep(-width, width, up);
    let sharpness = mix(PANEL_SHARPNESS, BLURRED_PANEL_SHARPNESS, roughness);
    let brightness = mix(PANEL_BRIGHTNESS, BLURRED_PANEL_BRIGHTNESS, roughness);
    let panel = pow(max(dot(reflected, normalize(PANEL_DIRECTION)), 0.0), sharpness) * brightness;
    return to_linear(mix(ground, sky, horizon)) + vec3<f32>(panel);
}

fn finished(color: vec4<f32>, normal: vec3<f32>, eye: vec3<f32>, finish: u32) -> vec4<f32> {
    let metalness = finish_metalness(finish);
    let roughness = finish_roughness(finish);
    let alpha = roughness * roughness;
    let base = to_linear(color.rgb);
    let reflectance = mix(vec3<f32>(DIELECTRIC_REFLECTANCE), min(base * METAL_REFLECTANCE_GAIN, vec3<f32>(1.0)), metalness);
    let facing = clamp(dot(normal, eye), 0.0, 1.0);
    let glancing = pow(1.0 - facing, 5.0);
    let mirrored = reflectance + (max(vec3<f32>(1.0 - roughness), reflectance) - reflectance) * glancing;
    let albedo = base * (1.0 - metalness * (1.0 - METAL_DIFFUSE)) * (vec3<f32>(1.0) - mirrored * (1.0 - metalness));

    var light = albedo * diffuse_light(normal, eye);
    if uses_enhanced_shading() {
        light += highlight(normal, eye, view.light.xyz, alpha, reflectance) * ENHANCED_KEY_LIGHT;
        light += highlight(normal, eye, view.fill_light.xyz, alpha, reflectance) * ENHANCED_FILL_LIGHT;
    } else {
        light += highlight(normal, eye, view.light.xyz, alpha, reflectance) * KEY_LIGHT;
    }
    light += studio(reflect(-eye, normal), roughness) * mirrored * ENVIRONMENT_STRENGTH;
    return vec4<f32>(to_srgb(light), color.a);
}

const SHADOW_TAPS: i32 = 1;
const BOUNCE_SAMPLES: i32 = 24;
const GOLDEN_ANGLE: f32 = 2.39996323;
const SHADOW_DEPTH_BIAS: f32 = 0.0005;
const ENVIRONMENT_BRDF_LOW: vec4<f32> = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
const ENVIRONMENT_BRDF_HIGH: vec4<f32> = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
const NEUTRAL_START: f32 = 0.76;
const NEUTRAL_DESATURATION: f32 = 0.15;

fn uses_rendered_shading() -> bool {
    return view.exposure_shadow.x > 0.0;
}

fn casts_shadows() -> bool {
    return view.exposure_shadow.y > 0.5;
}

fn sunlit(relative: vec3<f32>, normal: vec3<f32>) -> f32 {
    if !casts_shadows() {
        return 1.0;
    }
    let toward_light = view.sun.xyz;
    let slant = 1.0 - clamp(dot(normal, toward_light), 0.0, 1.0);
    let offset = normal * view.exposure_shadow.w * (0.5 + slant);
    let light = view.shadow_matrix * vec4<f32>(relative + offset, 1.0);
    let texel = vec2<f32>(light.x * 0.5 + 0.5, 0.5 - light.y * 0.5);
    if any(texel < vec2<f32>(0.0)) || any(texel > vec2<f32>(1.0)) || light.z > 1.0 || light.z < 0.0 {
        return 1.0;
    }
    let step = view.exposure_shadow.z;
    let reference = light.z - SHADOW_DEPTH_BIAS;
    var seen = 0.0;
    var taps = 0.0;
    for (var x = -SHADOW_TAPS; x <= SHADOW_TAPS; x++) {
        for (var y = -SHADOW_TAPS; y <= SHADOW_TAPS; y++) {
            let at = texel + vec2<f32>(f32(x), f32(y)) * step;
            seen += textureSampleCompareLevel(shadow_map, shadow_sampler, at, reference);
            taps += 1.0;
        }
    }
    return seen / taps;
}

fn visibility_term(n_dot_v: f32, n_dot_l: f32, alpha: f32) -> f32 {
    let squared = alpha * alpha;
    let from_view = n_dot_l * sqrt(n_dot_v * n_dot_v * (1.0 - squared) + squared);
    let from_light = n_dot_v * sqrt(n_dot_l * n_dot_l * (1.0 - squared) + squared);
    return 0.5 / max(from_view + from_light, 1e-5);
}

fn distribution_term(n_dot_h: f32, alpha: f32) -> f32 {
    let squared = alpha * alpha;
    let spread = n_dot_h * n_dot_h * (squared - 1.0) + 1.0;
    return squared / (PI * spread * spread);
}

fn environment_brdf(reflectance: vec3<f32>, roughness: f32, n_dot_v: f32) -> vec3<f32> {
    let fit = roughness * ENVIRONMENT_BRDF_LOW + ENVIRONMENT_BRDF_HIGH;
    let falloff = min(fit.x * fit.x, exp2(-9.28 * n_dot_v)) * fit.x + fit.y;
    let scale_bias = vec2<f32>(-1.04, 1.04) * falloff + fit.zw;
    return reflectance * scale_bias.x + vec3<f32>(scale_bias.y);
}

fn pbr_neutral(color: vec3<f32>) -> vec3<f32> {
    let least = min(color.r, min(color.g, color.b));
    let offset = select(0.04, least - 6.25 * least * least, least < 0.08);
    let shifted = color - vec3<f32>(offset);
    let peak = max(shifted.r, max(shifted.g, shifted.b));
    if peak < NEUTRAL_START {
        return shifted;
    }
    let span = 1.0 - NEUTRAL_START;
    let compressed = 1.0 - span * span / (peak + span - NEUTRAL_START);
    let scaled = shifted * (compressed / peak);
    let desaturation = 1.0 - 1.0 / (NEUTRAL_DESATURATION * (peak - compressed) + 1.0);
    return mix(scaled, vec3<f32>(compressed), desaturation);
}

struct Received {
    sun_seen: f32,
    bounce: vec3<f32>,
    occlusion: f32,
}

const OCCLUSION_SLICES: i32 = 2;
const OCCLUSION_STEPS: i32 = 6;
const OCCLUSION_FALLOFF: f32 = 0.615;
const OCCLUSION_DEPTH_MATCH: f32 = 0.02;
const OCCLUSION_BLUR_DEPTH: f32 = 0.05;
const OCCLUSION_BLUR_NORMAL: f32 = 0.8;
const HALF_PI: f32 = 1.5707963;

struct Surface {
    found: bool,
    position: vec3<f32>,
    normal: vec3<f32>,
}

fn packed_snorm(values: vec4<f32>) -> u32 {
    let scaled = vec4<i32>(floor(clamp(values, vec4<f32>(-1.0), vec4<f32>(1.0)) * 127.0 + 0.5));
    let bytes = bitcast<vec4<u32>>(scaled) & vec4<u32>(255u);
    return bytes.x | (bytes.y << 8u) | (bytes.z << 16u) | (bytes.w << 24u);
}

fn unpacked_snorm(packed: u32) -> vec4<f32> {
    let raised = vec4<u32>(packed << 24u, packed << 16u, packed << 8u, packed);
    let signed = bitcast<vec4<i32>>(raised) >> vec4<u32>(24u);
    return max(vec4<f32>(signed) / 127.0, vec4<f32>(-1.0));
}

fn surface_in(surfaces: texture_2d<u32>, texel: vec2<i32>) -> Surface {
    let size = vec2<i32>(textureDimensions(surfaces));
    if any(texel < vec2<i32>(0)) || any(texel >= size) {
        return Surface(false, vec3<f32>(0.0), vec3<f32>(0.0));
    }
    let raw = textureLoad(surfaces, texel, 0);
    return Surface(raw.w != 0u, bitcast<vec3<f32>>(raw.xyz), unpacked_snorm(raw.w).xyz);
}

fn occludes() -> bool {
    return view.occlusion.x > 0.0;
}

fn occlusion_at(pixel: vec2<f32>, relative: vec3<f32>) -> f32 {
    if !occludes() {
        return 1.0;
    }
    let texel = vec2<i32>(pixel + view.occlusion_frame.xy);
    let surface = surface_in(occlusion_surfaces, texel);
    let depth = view_depth(relative);
    if !surface.found || abs(view_depth(surface.position) - depth) > abs(depth) * OCCLUSION_DEPTH_MATCH {
        return 1.0;
    }
    return textureLoad(occlusion_map, texel, 0).r;
}

fn specular_occlusion(occlusion: f32, n_dot_v: f32, roughness: f32) -> f32 {
    return clamp(pow(n_dot_v + occlusion, exp2(-16.0 * roughness - 1.0)) - 1.0 + occlusion, 0.0, 1.0);
}

fn horizon_toward(texel: vec2<i32>, position: vec3<f32>, eye: vec3<f32>, lowest: f32) -> f32 {
    let sample = surface_in(surface_texels, texel);
    if !sample.found {
        return lowest;
    }
    let between = sample.position - position;
    let length_between = length(between);
    if length_between <= 1e-6 {
        return lowest;
    }
    let reach = view.occlusion.y;
    let falloff_range = OCCLUSION_FALLOFF * reach;
    let weight = clamp((reach - length_between) / falloff_range, 0.0, 1.0);
    return mix(lowest, dot(between / length_between, eye), weight);
}

fn bayer(pixel: vec2<u32>) -> f32 {
    var pattern = array<f32, 16>(
        0.0, 8.0, 2.0, 10.0,
        12.0, 4.0, 14.0, 6.0,
        3.0, 11.0, 1.0, 9.0,
        15.0, 7.0, 13.0, 5.0,
    );
    let cell = (pixel.y % 4u) * 4u + pixel.x % 4u;
    return (pattern[cell] + 0.5) / 16.0;
}

@vertex
fn vs_screen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - vec2<f32>(1.0), 0.0, 1.0);
}

@fragment
fn fs_surface(in: Varyings) -> @location(0) vec4<u32> {
    if in.color.a <= 0.0 || is_cut(in) {
        discard;
    }
    let normal = facing_normal(in, toward_eye(in.relative));
    return vec4<u32>(bitcast<vec3<u32>>(in.relative), packed_snorm(vec4<f32>(normal, 1.0)));
}

@fragment
fn fs_occlusion(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(at.xy);
    let centre = surface_in(surface_texels, texel);
    if !centre.found {
        return vec4<f32>(1.0);
    }
    let position = centre.position;
    let normal = normalize(centre.normal);
    let eye = toward_eye(position);
    let depth = max(view_depth(position), 1e-6);
    let pixel_size = select(view.occlusion.z * depth, view.occlusion.z, is_orthographic());
    let pixels = min(view.occlusion.y / max(pixel_size, 1e-12), view.occlusion_frame.z);
    if pixels < 1.0 {
        return vec4<f32>(1.0);
    }
    let noise = bayer(vec2<u32>(at.xy));
    var visibility = 0.0;
    for (var slice = 0; slice < OCCLUSION_SLICES; slice++) {
        let angle = (f32(slice) + noise) / f32(OCCLUSION_SLICES) * PI;
        let across = vec2<f32>(cos(angle), sin(angle));
        let along = view.camera_right.xyz * across.x - view.camera_up.xyz * across.y;
        let square = along - dot(along, eye) * eye;
        let axis = normalize(cross(square, eye));
        let projected = normal - axis * dot(normal, axis);
        let projected_length = length(projected);
        let cos_n = clamp(dot(projected, eye) / max(projected_length, 1e-6), -1.0, 1.0);
        let n = sign(dot(square, projected)) * acos(cos_n);
        let lowest_ahead = cos(n + HALF_PI);
        let lowest_behind = cos(n - HALF_PI);
        var ahead = lowest_ahead;
        var behind = lowest_behind;
        for (var step = 0; step < OCCLUSION_STEPS; step++) {
            let share = (f32(step) + fract(noise + f32(step) * 0.618034)) / f32(OCCLUSION_STEPS);
            let offset = across * max(share * share * pixels, f32(step) + 1.0);
            let reach = vec2<i32>(round(offset));
            ahead = max(ahead, horizon_toward(texel + reach, position, eye, lowest_ahead));
            behind = max(behind, horizon_toward(texel - reach, position, eye, lowest_behind));
        }
        let low = n + max(-acos(clamp(behind, -1.0, 1.0)) - n, -HALF_PI);
        let high = n + min(acos(clamp(ahead, -1.0, 1.0)) - n, HALF_PI);
        let arc = -cos(2.0 * low - n) + cos_n + 2.0 * low * sin(n)
            - cos(2.0 * high - n) + cos_n + 2.0 * high * sin(n);
        visibility += projected_length * 0.25 * arc;
    }
    visibility = clamp(visibility / f32(OCCLUSION_SLICES), 0.0, 1.0);
    let occlusion = clamp(1.0 - view.occlusion.x * (1.0 - visibility), 0.0, 1.0);
    return vec4<f32>(occlusion, 0.0, 0.0, 1.0);
}

@fragment
fn fs_occlusion_blur(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(at.xy);
    let centre = surface_in(surface_texels, texel);
    if !centre.found {
        return vec4<f32>(1.0);
    }
    let depth = view_depth(centre.position);
    let size = vec2<i32>(textureDimensions(raw_occlusion));
    var total = 0.0;
    var weight = 0.0;
    for (var x = -2; x <= 1; x++) {
        for (var y = -2; y <= 1; y++) {
            let at_texel = texel + vec2<i32>(x, y);
            let neighbour = surface_in(surface_texels, at_texel);
            let alike = neighbour.found
                && abs(view_depth(neighbour.position) - depth) <= abs(depth) * OCCLUSION_BLUR_DEPTH
                && dot(neighbour.normal, centre.normal) >= OCCLUSION_BLUR_NORMAL;
            if alike && all(at_texel < size) {
                total += textureLoad(raw_occlusion, at_texel, 0).r;
                weight += 1.0;
            }
        }
    }
    return vec4<f32>(select(1.0, total / weight, weight > 0.0), 0.0, 0.0, 1.0);
}

fn bounces() -> bool {
    return view.bounce.x > 0.0;
}

fn spin_at(pixel: vec2<f32>) -> f32 {
    return fract(52.9829189 * fract(dot(pixel, vec2<f32>(0.06711056, 0.00583715)))) * TAU;
}

fn bounced(relative: vec3<f32>, normal: vec3<f32>, pixel: vec2<f32>) -> vec3<f32> {
    if !bounces() {
        return vec3<f32>(0.0);
    }
    let light = view.shadow_matrix * vec4<f32>(relative, 1.0);
    let centre = vec2<f32>(light.x * 0.5 + 0.5, 0.5 - light.y * 0.5);
    let size = vec2<f32>(textureDimensions(lit_albedo));
    let last = vec2<i32>(size) - vec2<i32>(1);
    let spin = spin_at(pixel);
    var gathered = vec3<f32>(0.0);
    for (var index = 0; index < BOUNCE_SAMPLES; index++) {
        let share = (f32(index) + 0.5) / f32(BOUNCE_SAMPLES);
        let angle = f32(index) * GOLDEN_ANGLE + spin;
        let at = centre + vec2<f32>(cos(angle), sin(angle)) * view.bounce.y * sqrt(share);
        if any(at < vec2<f32>(0.0)) || any(at >= vec2<f32>(1.0)) {
            continue;
        }
        let texel = clamp(vec2<i32>(at * size), vec2<i32>(0), last);
        let albedo = textureLoad(lit_albedo, texel, 0);
        if albedo.a <= 0.0 {
            continue;
        }
        let depth = bitcast<f32>(textureLoad(lit_depth, texel, 0).r);
        let placed = view.relative_from_light * vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, depth, 1.0);
        let source = placed.xyz / placed.w;
        let source_normal = textureLoad(lit_normals, texel, 0).xyz * 2.0 - 1.0;
        let between = relative - source;
        let squared = max(dot(between, between), view.bounce.z);
        let leaving = max(dot(source_normal, between), 0.0);
        let arriving = max(dot(normal, -between), 0.0);
        gathered += to_linear(albedo.rgb) * albedo.a * leaving * arriving / (squared * squared);
    }
    return gathered * view.bounce.w * view.bounce.x * view.sun_colour.rgb * view.sun.w;
}

const SKY_ZENITH: vec3<f32> = vec3<f32>(0.1, 0.24, 0.6);
const SKY_HORIZON: vec3<f32> = vec3<f32>(0.62, 0.72, 0.86);
const SKY_GROUND: vec3<f32> = vec3<f32>(0.34, 0.33, 0.32);
const SKY_ABOVE: vec3<f32> = vec3<f32>(0.4, 0.52, 0.78);
const SKY_BELOW: vec3<f32> = vec3<f32>(0.33, 0.32, 0.31);
const BACKDROP_TOP: vec3<f32> = vec3<f32>(0.06, 0.17, 0.46);
const BACKDROP_BOTTOM: vec3<f32> = vec3<f32>(0.62, 0.74, 0.9);
const GROUND_SHADOW: f32 = 0.5;
const GROUND_FADE_FROM: f32 = 1.2;
const GROUND_FADE_TO: f32 = 3.5;

fn sky(direction: vec3<f32>) -> vec3<f32> {
    let up = direction.z;
    if up >= 0.0 {
        return mix(SKY_HORIZON, SKY_ZENITH, pow(up, 0.6));
    }
    return mix(SKY_HORIZON * 0.85, SKY_GROUND, smoothstep(0.0, 0.25, -up));
}

fn sky_irradiance(normal: vec3<f32>) -> vec3<f32> {
    return mix(SKY_BELOW, SKY_ABOVE, smoothstep(-1.0, 1.0, normal.z));
}

fn sky_reflection(reflected: vec3<f32>, roughness: f32) -> vec3<f32> {
    return mix(sky(reflected), sky_irradiance(reflected), roughness);
}

fn finished_colour(light: vec3<f32>) -> vec3<f32> {
    return to_srgb(pbr_neutral(light * view.exposure_shadow.x));
}

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> Varyings {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - vec2<f32>(1.0);
    var out = empty_varyings();
    out.position = vec4<f32>(corner, 0.0, 1.0);
    out.local = (corner - view.pick_transform.zw) / view.pick_transform.xy;
    return out;
}

@fragment
fn fs_sky(in: Varyings) -> @location(0) vec4<f32> {
    var ray = view.forward_near.xyz;
    if !is_orthographic() {
        ray = normalize(
            view.forward_near.xyz
                + view.camera_right.xyz * in.local.x * view.sky_view.x
                + view.camera_up.xyz * in.local.y * view.sky_view.y,
        );
    }
    let height = smoothstep(-1.0, 1.0, in.local.y);
    let backdrop = mix(BACKDROP_BOTTOM, BACKDROP_TOP, height * height);
    var shade = 1.0;
    let below = view.ground.x;
    if view.ground.y > 0.0 && view.sun.z > 0.0 && ray.z < -1e-4 && below < 0.0 {
        let hit = ray * (below / ray.z);
        let spread = length(hit.xy - view.ground.zw) / max(view.ground.y, 1e-6);
        let fade = smoothstep(GROUND_FADE_FROM, GROUND_FADE_TO, spread);
        let shadowed = 1.0 - sunlit(hit, vec3<f32>(0.0, 0.0, 1.0));
        shade = 1.0 - GROUND_SHADOW * shadowed * (1.0 - fade);
    }
    return vec4<f32>(to_srgb(pbr_neutral(backdrop) * shade), 1.0);
}

fn rendered(color: vec4<f32>, normal: vec3<f32>, eye: vec3<f32>, finish: u32, received: Received) -> vec4<f32> {
    let metalness = finish_metalness(finish);
    let roughness = finish_roughness(finish);
    let alpha = roughness * roughness;
    let base = to_linear(color.rgb);
    let reflectance = mix(vec3<f32>(DIELECTRIC_REFLECTANCE), base, metalness);
    let toward_light = view.sun.xyz;
    let n_dot_l = clamp(dot(normal, toward_light), 0.0, 1.0);
    let n_dot_v = clamp(dot(normal, eye), 1e-4, 1.0);

    let halfway = normalize(toward_light + eye);
    let n_dot_h = clamp(dot(normal, halfway), 0.0, 1.0);
    let v_dot_h = clamp(dot(eye, halfway), 0.0, 1.0);
    let fresnel = schlick(reflectance, v_dot_h);
    let specular = fresnel * distribution_term(n_dot_h, alpha) * visibility_term(n_dot_v, n_dot_l, alpha);
    let diffuse = (vec3<f32>(1.0) - fresnel) * (1.0 - metalness) * base / PI;
    let sunlight = view.sun_colour.rgb * view.sun.w * n_dot_l * received.sun_seen;
    let direct = (diffuse + specular) * sunlight;

    let reflected = environment_brdf(reflectance, roughness, n_dot_v);
    let diffuse_share = (vec3<f32>(1.0) - reflected) * (1.0 - metalness) * base;
    let occlusion = received.occlusion;
    let ambient_diffuse = diffuse_share * sky_irradiance(normal) * occlusion;
    let ambient_specular = reflected * sky_reflection(reflect(-eye, normal), roughness)
        * specular_occlusion(occlusion, n_dot_v, roughness);
    let ambient = (ambient_diffuse + ambient_specular) * view.sun_colour.w;
    let indirect = diffuse_share * received.bounce * occlusion / PI;

    return vec4<f32>(finished_colour(direct + ambient + indirect), color.a);
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

@fragment
fn fs_mesh_solid(in: Varyings) -> @location(0) vec4<f32> {
    let back = is_back_face(in);
    let hidden_inside = back && (in.sectioned & CAPPABLE) != 0u;
    if in.color.a <= 0.0 || is_cut(in) || hidden_inside {
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
    let finish = in.sectioned >> FINISH_SHIFT;
    let capped = vec4<f32>(in.color.rgb * CAP_SHADE, 1.0);
    if uses_rendered_shading() && (finish & FINISH_PRESENT) != 0u {
        return hatched(rendered(capped, normal, eye, finish, Received(1.0, vec3<f32>(0.0), 1.0)).rgb, section);
    }
    let filled = lit(capped, normal, eye);
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
    if in.color.a <= 0.0 || section.cut {
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

@vertex
fn vs_picture(@builtin(vertex_index) vertex: u32) -> PictureVaryings {
    var triangles = array<u32, 6>(0u, 1u, 2u, 0u, 2u, 3u);
    var texels = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(0.0, 0.0),
    );
    let corner = triangles[vertex % 6u];
    let position = from_anchor(picture.corners[corner].xyz);
    var out: PictureVaryings;
    out.position = finish(to_clip(position), MODEL_FILL_DEPTH_BIAS, BEHIND);
    out.texel = texels[corner];
    out.depth = view_depth(position);
    return out;
}

@fragment
fn fs_picture(in: PictureVaryings) -> @location(0) vec4<f32> {
    let sampled = textureSample(picture_texture, picture_sampler, in.texel);
    return vec4<f32>(sampled.rgb, sampled.a * picture.look.x);
}

@fragment
fn fs_picture_pick(in: PictureVaryings) -> PickOutput {
    if picture.pick.x == 0u {
        discard;
    }
    return PickOutput(picture.pick.x, bitcast<u32>(in.depth));
}
