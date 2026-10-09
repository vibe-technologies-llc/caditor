use caditor_geometry::{Rotation3, Vector3};
use egui::{
    Align2, Color32, CursorIcon, EventFilter, Key, PointerButton, Pos2, Rect, Sense, Shape, Stroke,
    Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

use crate::{canvas, commands::Command, icons, selection::Axis};

pub const NAME: &str = "View cube";
pub const HOME_NAME: &str = "Isometric view";

const CUBE_SIZE: f32 = 104.0;
const MARGIN: f32 = 12.0;
const PROJECTION_SCALE: f32 = 0.29;
const INNER_CELL_EXTENT: f64 = 0.55;
const MIN_VISIBLE_FACING: f64 = 0.02;
const FIT_BUTTON_GAP: f32 = 6.0;
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

fn cube_rect(viewport: Rect) -> Rect {
    Rect::from_min_size(
        pos2(
            viewport.right() - MARGIN - CUBE_SIZE,
            viewport.top() + MARGIN,
        ),
        vec2(CUBE_SIZE, CUBE_SIZE),
    )
}

fn fit_rect(viewport: Rect) -> Rect {
    let cube = cube_rect(viewport);
    Rect::from_min_size(
        pos2(cube.left(), cube.bottom() + FIT_BUTTON_GAP),
        vec2(CUBE_SIZE, canvas::BUTTON_HEIGHT),
    )
}

fn home_rect(viewport: Rect) -> Rect {
    let fit = fit_rect(viewport);
    Rect::from_min_size(
        pos2(
            fit.left() - FIT_BUTTON_GAP - canvas::BUTTON_HEIGHT,
            fit.top(),
        ),
        Vec2::splat(canvas::BUTTON_HEIGHT),
    )
}

pub fn area(viewport: Rect) -> Rect {
    cube_rect(viewport)
        .union(fit_rect(viewport))
        .union(home_rect(viewport))
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
    viewport: Rect,
    orientation: Rotation3,
    texts: &CubeTexts<'_>,
) -> Option<CubeAction> {
    let rect = cube_rect(viewport);
    let id = ui.id().with("view cube");
    let response = ui.interact(rect, id, Sense::click_and_drag());
    let orbiting = response.dragged_by(PointerButton::Primary);
    let projector = Projector {
        to_view: orientation.inverse(),
        center: rect.center(),
        scale: CUBE_SIZE * PROJECTION_SCALE,
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
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, ui.is_enabled(), &name));
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

    let lit = |direction: Vector3| {
        (hovered == Some(direction)).then_some(if pressed {
            canvas::CUBE_PRESSED
        } else {
            canvas::CUBE_HOVERED
        })
    };
    let painter = ui.painter_at(rect.expand(PAINT_ROOM));
    for cell in &cells {
        let fill = lit(cell.direction).unwrap_or_else(|| canvas::cube_face(cell.facing));
        painter.add(Shape::convex_polygon(
            cell.corners.to_vec(),
            fill,
            Stroke::new(DIVIDER_WIDTH, canvas::CUBE_DIVIDER),
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
            Stroke::new(EDGE_WIDTH, canvas::CUBE_EDGE),
        ));
        if facing > canvas::CUBE_DIMMEST_LABELLED_FACING {
            let color = if lit(face.normal).is_some() {
                canvas::CUBE_LABEL_ON_HOVER
            } else {
                canvas::CUBE_LABEL
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
        canvas::paint_focus_ring(ui.painter(), rect);
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
        fit_rect(viewport),
        ui.id().with("fit view"),
        icons::command(Command::FitView),
        texts.fit_label,
    )
    .on_hover_text(texts.fit_hover);
    let home = canvas::icon_button(
        ui,
        home_rect(viewport),
        ui.id().with("home view"),
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

pub fn show_axis_triad(ui: &Ui, viewport: Rect, orientation: Rotation3) {
    let projector = Projector {
        to_view: orientation.inverse(),
        center: pos2(
            viewport.left() + MARGIN + TRIAD_OFFSET,
            viewport.bottom() - MARGIN - TRIAD_OFFSET,
        ),
        scale: TRIAD_LENGTH,
    };
    let mut axes: Vec<(f64, Axis)> = Axis::ALL
        .into_iter()
        .map(|axis| (projector.facing(axis.direction()), axis))
        .collect();
    axes.sort_by(|a, b| a.0.total_cmp(&b.0));

    let painter = ui.painter();
    for (_, axis) in axes {
        let [red, green, blue] = axis.rgb();
        let color = Color32::from_rgb(red, green, blue);
        let tip = projector.project(axis.direction());
        let label = projector.project(axis.direction() * f64::from(TRIAD_LABEL_REACH));
        painter.line_segment([projector.center, tip], Stroke::new(TRIAD_STROKE, color));
        painter.text(
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

    use super::*;

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
        let area = area(viewport);
        assert!(area.contains_rect(cube_rect(viewport)));
        assert!(area.contains_rect(fit_rect(viewport)));
        assert!(fit_rect(viewport).top() > cube_rect(viewport).bottom());
        assert_eq!(fit_rect(viewport).height(), canvas::BUTTON_HEIGHT);
        assert!(area.contains_rect(home_rect(viewport)));
        assert!(home_rect(viewport).right() < fit_rect(viewport).left());
        assert_eq!(home_rect(viewport).top(), fit_rect(viewport).top());
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
}
