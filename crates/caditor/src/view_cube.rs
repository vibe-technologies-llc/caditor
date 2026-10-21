use caditor_geometry::{Rotation3, Vector3};
use egui::{
    Align2, CursorIcon, EventFilter, Key, PointerButton, Pos2, Rect, Role, Sense, Shape, Stroke,
    Ui, Vec2, WidgetInfo, pos2, vec2,
};

use crate::{
    appearance::{FOCUS_GAP, SPACE_SM},
    canvas,
    commands::Command,
    icons,
    scene_palette::{self, ScenePalette},
    selection::Axis,
    widgets,
};

pub const NAME: &str = "View cube";
pub const HOME_NAME: &str = "Isometric view";

const SMALL_CUBE: f32 = 80.0;
const STANDARD_CUBE: f32 = 104.0;
const LARGE_CUBE: f32 = 136.0;
const MARGIN: f32 = canvas::MARGIN;
const PROJECTION_SCALE: f32 = 0.29;
const INNER_CELL_EXTENT: f64 = 0.55;
const MIN_VISIBLE_FACING: f64 = 0.02;
const FIT_BUTTON_GAP: f32 = SPACE_SM;
const DIVIDER_WIDTH: f32 = 1.0;
const EDGE_WIDTH: f32 = 1.0;
const PAINT_ROOM: f32 = 2.0;
const SIDE_COMPONENT: f64 = 0.5;
const NEIGHBOUR_MIN_COSINE: f64 = 0.5;
const STEP_MIN_ALIGNMENT: f32 = 0.5;
const SAME_DIRECTION: f64 = 1.0 - 1e-9;
const TRIAD_LENGTH: f32 = 26.0;
const TRIAD_OFFSET: f32 = 34.0;
const TRIAD_LABEL_REACH: f32 = 1.35;
const TRIAD_LABEL_ROOM: f32 = 8.0;
const TRIAD_STROKE: f32 = 2.0;
const TRIAD_HALO: f32 = 1.0;

struct Face {
    normal: Vector3,
    u: Vector3,
    v: Vector3,
    label: &'static str,
}

const FACES: [Face; 6] = [
    Face {
        normal: Vector3::Z,
        u: Vector3::X,
        v: Vector3::Y,
        label: "Top",
    },
    Face {
        normal: Vector3::NEG_Z,
        u: Vector3::X,
        v: Vector3::NEG_Y,
        label: "Bottom",
    },
    Face {
        normal: Vector3::NEG_Y,
        u: Vector3::X,
        v: Vector3::Z,
        label: "Front",
    },
    Face {
        normal: Vector3::Y,
        u: Vector3::NEG_X,
        v: Vector3::Z,
        label: "Back",
    },
    Face {
        normal: Vector3::X,
        u: Vector3::Y,
        v: Vector3::Z,
        label: "Right",
    },
    Face {
        normal: Vector3::NEG_X,
        u: Vector3::NEG_Y,
        v: Vector3::Z,
        label: "Left",
    },
];

const CELL_SPANS: [(i8, f64, f64); 3] = [
    (-1, -1.0, -INNER_CELL_EXTENT),
    (0, -INNER_CELL_EXTENT, INNER_CELL_EXTENT),
    (1, INNER_CELL_EXTENT, 1.0),
];

const ARROWS: [(Key, Vec2); 4] = [
    (Key::ArrowLeft, vec2(-1.0, 0.0)),
    (Key::ArrowRight, vec2(1.0, 0.0)),
    (Key::ArrowUp, vec2(0.0, -1.0)),
    (Key::ArrowDown, vec2(0.0, 1.0)),
];

pub enum CubeAction {
    LookFrom(Vector3),
    Orbit(Vec2),
    Fit,
    Home,
}

pub struct CubeTexts<'a> {
    pub fit_label: &'a str,
    pub fit_hover: &'a str,
    pub home_hover: &'a str,
    pub view_keys: &'a [(Vector3, String)],
}

impl CubeTexts<'_> {
    fn hover(&self, direction: Vector3) -> String {
        let name = view_name(direction);
        let toward = direction.normalize_or_zero();
        match self
            .view_keys
            .iter()
            .find(|(looking_from, _)| looking_from.normalize_or_zero().dot(toward) > SAME_DIRECTION)
        {
            Some((_, keys)) => format!("{name} ({keys})"),
            None => name,
        }
    }
}

struct Cell {
    direction: Vector3,
    corners: [Pos2; 4],
    facing: f64,
}

struct Projector {
    to_view: Rotation3,
    center: Pos2,
    scale: f32,
}

impl Projector {
    fn project(&self, point: Vector3) -> Pos2 {
        self.center + self.on_screen(point) * self.scale
    }

    fn on_screen(&self, direction: Vector3) -> Vec2 {
        let local = self.to_view * direction;
        vec2(local.x as f32, -(local.y as f32))
    }

    fn facing(&self, normal: Vector3) -> f64 {
        (self.to_view * normal).z
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CubeSize {
    Small,
    #[default]
    Standard,
    Large,
}

impl CubeSize {
    pub fn side(self) -> f32 {
        match self {
            Self::Small => SMALL_CUBE,
            Self::Standard => STANDARD_CUBE,
            Self::Large => LARGE_CUBE,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    pub fn is_top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight)
    }

    pub fn is_left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft)
    }

    pub fn point(self, viewport: Rect) -> Pos2 {
        let inner = viewport.shrink(MARGIN);
        pos2(
            if self.is_left() {
                inner.left()
            } else {
                inner.right()
            },
            if self.is_top() {
                inner.top()
            } else {
                inner.bottom()
            },
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CubePlacement {
    pub size: CubeSize,
    pub corner: Corner,
}

impl CubePlacement {
    fn side(self) -> f32 {
        self.size.side()
    }

    fn fit_width(self) -> f32 {
        self.side().max(STANDARD_CUBE)
    }

    fn column(self, viewport: Rect, width: f32) -> f32 {
        let corner = self.corner.point(viewport);
        if self.corner.is_left() {
            corner.x
        } else {
            corner.x - width
        }
    }

    fn cube_rect(self, viewport: Rect) -> Rect {
        let side = self.side();
        let controls = canvas::BUTTON_HEIGHT + FIT_BUTTON_GAP;
        let corner = self.corner.point(viewport);
        let top = if self.corner.is_top() {
            corner.y
        } else {
            corner.y - side - controls
        };
        Rect::from_min_size(pos2(self.column(viewport, side), top), vec2(side, side))
    }

    fn fit_rect(self, viewport: Rect) -> Rect {
        let cube = self.cube_rect(viewport);
        let width = self.fit_width();
        Rect::from_min_size(
            pos2(self.column(viewport, width), cube.bottom() + FIT_BUTTON_GAP),
            vec2(width, canvas::BUTTON_HEIGHT),
        )
    }

    fn home_rect(self, viewport: Rect) -> Rect {
        let fit = self.fit_rect(viewport);
        let left = if self.corner.is_left() {
            fit.right() + FIT_BUTTON_GAP
        } else {
            fit.left() - FIT_BUTTON_GAP - canvas::BUTTON_HEIGHT
        };
        Rect::from_min_size(pos2(left, fit.top()), Vec2::splat(canvas::BUTTON_HEIGHT))
    }

    pub fn area(self, viewport: Rect) -> Rect {
        self.cube_rect(viewport)
            .union(self.fit_rect(viewport))
            .union(self.home_rect(viewport))
    }
}

#[cfg(test)]
pub fn area(viewport: Rect) -> Rect {
    CubePlacement::default().area(viewport)
}

pub fn view_name(direction: Vector3) -> String {
    let sides: Vec<&str> = [
        (direction.z, "top", "bottom"),
        (-direction.y, "front", "back"),
        (direction.x, "right", "left"),
    ]
    .into_iter()
    .filter_map(|(along, positive, negative)| {
        if along > SIDE_COMPONENT {
            Some(positive)
        } else if along < -SIDE_COMPONENT {
            Some(negative)
        } else {
            None
        }
    })
    .collect();
    match sides.as_slice() {
        [first, second, third] => format!("View from {first}, {second} and {third}"),
        [first, second] => format!("View from {first} and {second}"),
        [only] => format!("View from {only}"),
        _ => "View".to_owned(),
    }
}

pub fn show(
    ui: &mut Ui,
    (viewport, placement): (Rect, CubePlacement),
    orientation: Rotation3,
    texts: &CubeTexts<'_>,
) -> Option<CubeAction> {
    let rect = placement.cube_rect(viewport);
    let id = ui.scope_id().with("view cube");
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let orbiting = response.dragged_by(PointerButton::Primary);
    let projector = Projector {
        to_view: orientation.inverse(),
        center: rect.center(),
        scale: placement.side() * PROJECTION_SCALE,
    };
    let cells = visible_cells(&projector);
    let hovered = response
        .hover_pos()
        .filter(|_| !orbiting)
        .and_then(|pointer| cells.iter().find(|cell| contains(&cell.corners, pointer)))
        .map(|cell| cell.direction);
    let pressed = response.is_pointer_button_down_on();
    let current = nearest_target(orientation * Vector3::Z);
    let target = hovered.unwrap_or(current);
    let name = format!("{NAME}: {}", view_name(target));
    response.widget_info(|| WidgetInfo::labeled(Role::Button, ui.is_enabled(), &name));
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                id,
                EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..EventFilter::default()
                },
            );
        });
    }
    let stepped = focused
        .then(|| {
            ARROWS
                .into_iter()
                .find(|(key, _)| ui.input(|input| input.key_pressed(*key)))
        })
        .flatten()
        .and_then(|(_, step)| neighbour(&projector, current, step));

    let chrome = canvas::chrome(ui.ctx());
    let lit = |direction: Vector3| {
        (hovered == Some(direction)).then_some(if pressed {
            chrome.cube_pressed
        } else {
            chrome.cube_hovered
        })
    };
    let painter = ui.painter_at(rect.expand(PAINT_ROOM));
    for cell in &cells {
        let fill = lit(cell.direction).unwrap_or_else(|| chrome.cube_face(cell.facing));
        painter.add(Shape::convex_polygon(
            cell.corners.to_vec(),
            fill,
            Stroke::new(DIVIDER_WIDTH, chrome.cube_divider),
        ));
    }
    for face in &FACES {
        let facing = projector.facing(face.normal);
        if facing <= MIN_VISIBLE_FACING {
            continue;
        }
        let outline = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(a, b)| projector.project(face.normal + face.u * a + face.v * b));
        painter.add(Shape::closed_line(
            outline.to_vec(),
            Stroke::new(EDGE_WIDTH, chrome.cube_edge),
        ));
        if facing > canvas::CUBE_DIMMEST_LABELLED_FACING {
            let color = if lit(face.normal).is_some() {
                chrome.cube_label_on_hover
            } else {
                chrome.cube_label
            };
            painter.text(
                projector.project(face.normal),
                Align2::CENTER_CENTER,
                face.label,
                canvas::emphasis(),
                color,
            );
        }
    }
    if focused {
        widgets::paint_focus_ring_in(
            ui.painter(),
            rect.expand(FOCUS_GAP),
            canvas::RADIUS,
            chrome.focus,
        );
    }
    if orbiting {
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    } else if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let response = match hovered {
        Some(direction) => response.on_hover_text(texts.hover(direction)),
        None => response,
    };

    let fit = canvas::button(
        ui,
        placement.fit_rect(viewport),
        ui.scope_id().with("fit view"),
        icons::command(Command::FitView),
        texts.fit_label,
    )
    .on_hover_text(texts.fit_hover);
    let home = canvas::icon_button(
        ui,
        placement.home_rect(viewport),
        ui.scope_id().with("home view"),
        icons::HOME_VIEW,
        HOME_NAME,
    )
    .on_hover_text(texts.home_hover);

    if fit.clicked() {
        Some(CubeAction::Fit)
    } else if home.clicked() {
        Some(CubeAction::Home)
    } else if orbiting {
        let drag = response.drag_delta();
        (drag != Vec2::ZERO).then_some(CubeAction::Orbit(drag))
    } else if response.clicked() && hovered.is_some() {
        hovered.map(CubeAction::LookFrom)
    } else {
        stepped.map(CubeAction::LookFrom)
    }
}

fn targets() -> impl Iterator<Item = Vector3> {
    (-1..=1).flat_map(|x| {
        (-1..=1).flat_map(move |y| {
            (-1..=1)
                .map(move |z| Vector3::new(f64::from(x), f64::from(y), f64::from(z)))
                .filter(|direction| *direction != Vector3::ZERO)
        })
    })
}

fn nearest_target(looking_from: Vector3) -> Vector3 {
    let toward = looking_from.normalize_or_zero();
    targets()
        .max_by(|a, b| {
            a.normalize()
                .dot(toward)
                .total_cmp(&b.normalize().dot(toward))
        })
        .unwrap_or(Vector3::Z)
}

fn neighbour(projector: &Projector, current: Vector3, step: Vec2) -> Option<Vector3> {
    let from = current.normalize_or_zero();
    targets()
        .filter(|target| *target != current)
        .filter_map(|target| {
            let closeness = target.normalize().dot(from);
            let moved = projector.on_screen(target.normalize() - from).normalized();
            let alignment = moved.dot(step);
            (closeness >= NEIGHBOUR_MIN_COSINE && alignment >= STEP_MIN_ALIGNMENT)
                .then_some((target, alignment, closeness))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)))
        .map(|(target, _, _)| target)
}

pub const TRIAD_WIDTH: f32 =
    MARGIN + TRIAD_OFFSET + TRIAD_LENGTH * TRIAD_LABEL_REACH + TRIAD_LABEL_ROOM;

pub fn show_axis_triad(ui: &Ui, bottom_left: Pos2, orientation: Rotation3, palette: &ScenePalette) {
    let projector = Projector {
        to_view: orientation.inverse(),
        center: pos2(
            bottom_left.x + MARGIN + TRIAD_OFFSET,
            bottom_left.y - MARGIN - TRIAD_OFFSET,
        ),
        scale: TRIAD_LENGTH,
    };
    let mut axes: Vec<(f64, Axis)> = Axis::ALL
        .into_iter()
        .map(|axis| (projector.facing(axis.direction()), axis))
        .collect();
    axes.sort_by(|a, b| a.0.total_cmp(&b.0));

    let painter = ui.painter();
    for (_, axis) in &axes {
        let tip = projector.project(axis.direction());
        painter.line_segment(
            [projector.center, tip],
            Stroke::new(
                TRIAD_STROKE + 2.0 * TRIAD_HALO,
                scene_palette::color32(palette.axis_halo),
            ),
        );
        painter.line_segment(
            [projector.center, tip],
            Stroke::new(TRIAD_STROKE, palette.axis_colour(*axis)),
        );
    }
    for (_, axis) in axes {
        let color = palette.axis_colour(axis);
        let label = projector.project(axis.direction() * f64::from(TRIAD_LABEL_REACH));
        canvas::label(
            painter,
            label,
            Align2::CENTER_CENTER,
            axis.letter(),
            canvas::emphasis(),
            color,
        );
    }
}

fn visible_cells(projector: &Projector) -> Vec<Cell> {
    let mut cells = Vec::new();
    for face in &FACES {
        let facing = projector.facing(face.normal);
        if facing <= MIN_VISIBLE_FACING {
            continue;
        }
        for (i, u_start, u_end) in CELL_SPANS {
            for (j, v_start, v_end) in CELL_SPANS {
                let corner =
                    |a: f64, b: f64| projector.project(face.normal + face.u * a + face.v * b);
                cells.push(Cell {
                    direction: face.normal + face.u * f64::from(i) + face.v * f64::from(j),
                    corners: [
                        corner(u_start, v_start),
                        corner(u_end, v_start),
                        corner(u_end, v_end),
                        corner(u_start, v_end),
                    ],
                    facing,
                });
            }
        }
    }
    cells
}

fn contains(corners: &[Pos2; 4], point: Pos2) -> bool {
    let mut sign = 0.0f32;
    for (index, start) in corners.iter().enumerate() {
        let Some(end) = corners.get((index + 1) % corners.len()) else {
            return false;
        };
        let cross = (*end - *start).x * (point - *start).y - (*end - *start).y * (point - *start).x;
        if cross.abs() <= f32::EPSILON {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use caditor_geometry::Point3;
    use caditor_render::Viewpoint;
    use egui::{Color32, RawInput, epaint::Shape};

    use super::*;
    use crate::scene_palette::{Canvas, Contrast};

    fn triad_shapes(contrast: Contrast, canvas: Canvas) -> Vec<Shape> {
        let context = egui::Context::default();
        context.set_fonts(crate::fonts::definitions_with(&[]));
        canvas::set_contrast(&context, contrast);
        canvas::set_canvas(&context, canvas);
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let orientation = Viewpoint::looking_from(Vector3::new(1.0, -1.0, 1.0), Point3::ZERO, 1.0)
            .unwrap()
            .orientation;
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut output = context.run_ui(
                RawInput {
                    screen_rect: Some(viewport),
                    ..RawInput::default()
                },
                |ui| {
                    show_axis_triad(
                        ui,
                        viewport.left_bottom(),
                        orientation,
                        contrast.palette(canvas),
                    )
                },
            );
            output.textures_delta.clear();
            shapes = output
                .shapes
                .into_iter()
                .map(|clipped| clipped.shape)
                .collect();
        }
        shapes
    }

    fn projector_looking_from(direction: Vector3) -> Projector {
        let viewpoint = Viewpoint::looking_from(direction, Point3::ZERO, 1.0).unwrap();
        Projector {
            to_view: viewpoint.orientation.inverse(),
            center: pos2(0.0, 0.0),
            scale: 10.0,
        }
    }

    #[test]
    fn a_front_view_shows_only_the_front_face_and_its_centre_looks_from_the_front() {
        let projector = projector_looking_from(Vector3::NEG_Y);
        let cells = visible_cells(&projector);

        assert_eq!(cells.len(), 9);
        let center = cells
            .iter()
            .find(|cell| contains(&cell.corners, pos2(0.0, 0.0)))
            .unwrap();
        assert_eq!(center.direction, Vector3::NEG_Y);

        let top_right = cells
            .iter()
            .find(|cell| contains(&cell.corners, pos2(9.0, -9.0)))
            .unwrap();
        assert_eq!(top_right.direction, Vector3::new(1.0, -1.0, 1.0));
    }

    #[test]
    fn an_isometric_view_shows_three_faces() {
        let projector = projector_looking_from(Vector3::new(1.0, -1.0, 1.0));
        assert_eq!(visible_cells(&projector).len(), 27);
    }

    #[test]
    fn containment_accepts_either_winding() {
        let square = [
            pos2(0.0, 0.0),
            pos2(1.0, 0.0),
            pos2(1.0, 1.0),
            pos2(0.0, 1.0),
        ];
        let mut reversed = square;
        reversed.reverse();
        for corners in [square, reversed] {
            assert!(contains(&corners, pos2(0.5, 0.5)));
            assert!(!contains(&corners, pos2(1.5, 0.5)));
        }
    }

    #[test]
    fn every_target_is_named_by_the_sides_it_looks_from() {
        assert_eq!(view_name(Vector3::Z), "View from top");
        assert_eq!(
            view_name(Vector3::new(0.0, -1.0, 1.0)),
            "View from top and front"
        );
        assert_eq!(
            view_name(Vector3::new(-1.0, 1.0, -1.0)),
            "View from bottom, back and left"
        );
        let names: Vec<String> = targets().map(view_name).collect();
        assert_eq!(names.len(), 26);
        for (index, name) in names.iter().enumerate() {
            assert!(!names[..index].contains(name), "{name} is named twice");
        }
    }

    #[test]
    fn arrows_step_to_the_neighbouring_target_on_that_side_of_the_cube() {
        let front = projector_looking_from(Vector3::NEG_Y);
        assert_eq!(
            neighbour(&front, Vector3::NEG_Y, vec2(1.0, 0.0)),
            Some(Vector3::new(1.0, -1.0, 0.0))
        );
        assert_eq!(
            neighbour(&front, Vector3::NEG_Y, vec2(0.0, -1.0)),
            Some(Vector3::new(0.0, -1.0, 1.0))
        );
        let edge = projector_looking_from(Vector3::new(1.0, -1.0, 0.0));
        assert_eq!(
            neighbour(&edge, Vector3::new(1.0, -1.0, 0.0), vec2(1.0, 0.0)),
            Some(Vector3::X)
        );
        assert_eq!(
            nearest_target(Vector3::new(0.9, -1.1, 1.0)),
            Vector3::new(1.0, -1.0, 1.0)
        );
    }

    #[test]
    fn the_fit_button_sits_under_the_cube_inside_its_area() {
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let placement = CubePlacement::default();
        let area = area(viewport);
        let cube = placement.cube_rect(viewport);
        let fit = placement.fit_rect(viewport);
        let home = placement.home_rect(viewport);

        assert_eq!(placement.area(viewport), area);
        assert_eq!(cube.right_top(), pos2(800.0 - MARGIN, MARGIN));
        assert_eq!(cube.width(), STANDARD_CUBE);
        assert!(area.contains_rect(cube));
        assert!(area.contains_rect(fit));
        assert!(fit.top() > cube.bottom());
        assert_eq!(fit.height(), canvas::BUTTON_HEIGHT);
        assert!(area.contains_rect(home));
        assert!(home.right() < fit.left());
        assert_eq!(home.top(), fit.top());
    }

    #[test]
    fn every_size_and_corner_keeps_the_cube_and_its_buttons_inside_the_view_in_its_corner() {
        let viewport = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        for size in [CubeSize::Small, CubeSize::Standard, CubeSize::Large] {
            for corner in Corner::ALL {
                let placement = CubePlacement { size, corner };
                let area = placement.area(viewport);
                let cube = placement.cube_rect(viewport);
                let fit = placement.fit_rect(viewport);
                let home = placement.home_rect(viewport);
                let at = corner.point(viewport);

                assert!(
                    viewport.shrink(MARGIN - 0.01).contains_rect(area),
                    "{placement:?}"
                );
                assert_eq!(cube.width(), size.side());
                assert!(
                    fit.top() > cube.bottom() && !fit.intersects(home),
                    "{placement:?}"
                );
                assert!(fit.width() >= STANDARD_CUBE);
                assert!(
                    (area.left() - at.x).abs() < 0.01 || (area.right() - at.x).abs() < 0.01,
                    "{placement:?} leaves its corner"
                );
                assert!(
                    (area.top() - at.y).abs() < 0.01 || (area.bottom() - at.y).abs() < 0.01,
                    "{placement:?} leaves its corner"
                );
                assert_eq!(corner.is_left(), area.center().x < viewport.center().x);
                assert_eq!(corner.is_top(), area.center().y < viewport.center().y);
            }
        }
        assert!(CubeSize::Small.side() < CubeSize::Standard.side());
        assert!(CubeSize::Standard.side() < CubeSize::Large.side());
    }

    #[test]
    fn a_standard_direction_names_its_keys_on_hover() {
        let keys = [
            (Vector3::NEG_Y, "Alt+1".to_owned()),
            (Vector3::new(1.0, -1.0, 1.0), "Alt+0".to_owned()),
        ];
        let texts = CubeTexts {
            fit_label: "Fit all",
            fit_hover: "",
            home_hover: "",
            view_keys: &keys,
        };

        assert_eq!(texts.hover(Vector3::NEG_Y), "View from front (Alt+1)");
        assert_eq!(
            texts.hover(Vector3::new(1.0, -1.0, 1.0)),
            "View from top, front and right (Alt+0)"
        );
        assert_eq!(
            texts.hover(Vector3::new(1.0, -1.0, 0.0)),
            "View from front and right"
        );
    }

    #[test]
    fn the_axis_triad_draws_its_lines_and_letters_from_the_scene_palette_on_the_label_backdrop() {
        for contrast in [Contrast::Standard, Contrast::High] {
            for canvas in Canvas::ALL {
                let palette = contrast.palette(canvas);
                let backdrop = canvas.chrome().backdrop(contrast);
                let shapes = triad_shapes(contrast, canvas);

                let strokes: Vec<_> = shapes
                    .iter()
                    .filter_map(|shape| match shape {
                        Shape::LineSegment { stroke, .. } => Some(*stroke),
                        _ => None,
                    })
                    .collect();
                let mut lines: Vec<Color32> = strokes
                    .iter()
                    .filter(|stroke| stroke.width == TRIAD_STROKE)
                    .map(|stroke| stroke.color)
                    .collect();
                let halos: Vec<Color32> = strokes
                    .iter()
                    .filter(|stroke| stroke.width == TRIAD_STROKE + 2.0 * TRIAD_HALO)
                    .map(|stroke| stroke.color)
                    .collect();
                let mut letters: Vec<Color32> = shapes
                    .iter()
                    .filter_map(|shape| match shape {
                        Shape::Text(text) => Some(text.fallback_color),
                        _ => None,
                    })
                    .collect();
                let backdrops = shapes
                    .iter()
                    .filter(|shape| matches!(shape, Shape::Rect(rect) if rect.fill == backdrop))
                    .count();
                let mut expected: Vec<Color32> = Axis::ALL
                    .into_iter()
                    .map(|axis| palette.axis_colour(axis))
                    .collect();

                lines.sort_by_key(|colour| colour.to_array());
                letters.sort_by_key(|colour| colour.to_array());
                expected.sort_by_key(|colour| colour.to_array());

                assert_eq!(lines, expected, "{contrast:?} {canvas:?} lines");
                assert_eq!(
                    halos,
                    vec![scene_palette::color32(palette.axis_halo); 3],
                    "{contrast:?} {canvas:?} halos"
                );
                assert_eq!(letters, expected, "{contrast:?} {canvas:?} letters");
                assert_eq!(backdrops, 3, "{contrast:?} {canvas:?} backdrops");
            }
        }
    }

    #[test]
    fn the_axis_triad_draws_every_letter_over_every_line() {
        for contrast in [Contrast::Standard, Contrast::High] {
            let shapes = triad_shapes(contrast, Canvas::ALL[0]);

            let last_line = shapes
                .iter()
                .rposition(|shape| matches!(shape, Shape::LineSegment { .. }));
            let first_letter = shapes
                .iter()
                .position(|shape| matches!(shape, Shape::Text(_)));
            let first_backdrop = shapes
                .iter()
                .position(|shape| matches!(shape, Shape::Rect(_)));

            assert!(last_line.is_some(), "{contrast:?}");
            assert!(last_line < first_letter, "{contrast:?}");
            assert!(last_line < first_backdrop, "{contrast:?}");
        }
    }
}
