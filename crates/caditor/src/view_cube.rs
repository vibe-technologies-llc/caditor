use caditor_geometry::{Rotation3, Vector3};
use egui::{Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};

use crate::selection::Axis;

const CUBE_SIZE: f32 = 104.0;
const MARGIN: f32 = 12.0;
const PROJECTION_SCALE: f32 = 0.29;
const INNER_CELL_EXTENT: f64 = 0.55;
const MIN_VISIBLE_FACING: f64 = 0.02;
const MIN_LABEL_FACING: f64 = 0.3;
const FIT_BUTTON_HEIGHT: f32 = 22.0;
const TRIAD_LENGTH: f32 = 26.0;
const TRIAD_OFFSET: f32 = 34.0;

const FACE_FILL: [u8; 3] = [58, 64, 76];
const FACE_HOVER: Color32 = Color32::from_rgb(255, 196, 84);
const FACE_EDGE: Color32 = Color32::from_rgb(120, 130, 150);
const LABEL: [u8; 3] = [225, 228, 235];

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

pub enum CubeAction {
    LookFrom(Vector3),
    Fit,
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
        let local = self.to_view * point;
        self.center + vec2(local.x as f32, -(local.y as f32)) * self.scale
    }

    fn facing(&self, normal: Vector3) -> f64 {
        (self.to_view * normal).z
    }
}

pub fn show(
    ui: &mut egui::Ui,
    viewport: Rect,
    orientation: Rotation3,
    fit_label: &str,
    fit_hover: &str,
) -> Option<CubeAction> {
    let rect = Rect::from_min_size(
        pos2(
            viewport.right() - MARGIN - CUBE_SIZE,
            viewport.top() + MARGIN,
        ),
        vec2(CUBE_SIZE, CUBE_SIZE),
    );
    let response = ui.interact(rect, ui.id().with("view cube"), Sense::click());
    let projector = Projector {
        to_view: orientation.inverse(),
        center: rect.center(),
        scale: CUBE_SIZE * PROJECTION_SCALE,
    };
    let cells = visible_cells(&projector);
    let hovered = response
        .hover_pos()
        .and_then(|pointer| cells.iter().find(|cell| contains(&cell.corners, pointer)))
        .map(|cell| cell.direction);

    let painter = ui.painter_at(rect.expand(2.0));
    for cell in &cells {
        let fill = if hovered == Some(cell.direction) {
            FACE_HOVER
        } else {
            shade(FACE_FILL, cell.facing)
        };
        painter.add(Shape::convex_polygon(
            cell.corners.to_vec(),
            fill,
            Stroke::NONE,
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
            Stroke::new(1.0, FACE_EDGE),
        ));
        if facing > MIN_LABEL_FACING {
            let [red, green, blue] = LABEL;
            let alpha = (facing.clamp(0.0, 1.0) * 255.0) as u8;
            painter.text(
                projector.project(face.normal),
                Align2::CENTER_CENTER,
                face.label,
                FontId::proportional(12.0),
                Color32::from_rgba_unmultiplied(red, green, blue, alpha),
            );
        }
    }

    if hovered.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let fit_rect = Rect::from_min_size(
        pos2(rect.left(), rect.bottom() + 6.0),
        vec2(CUBE_SIZE, FIT_BUTTON_HEIGHT),
    );
    let fit = ui
        .put(fit_rect, egui::Button::new(fit_label))
        .on_hover_text(fit_hover);

    if fit.clicked() {
        Some(CubeAction::Fit)
    } else if response.clicked() {
        hovered.map(CubeAction::LookFrom)
    } else {
        None
    }
}

pub fn show_axis_triad(ui: &egui::Ui, viewport: Rect, orientation: Rotation3) {
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
        let label = projector.project(axis.direction() * 1.35);
        painter.line_segment([projector.center, tip], Stroke::new(2.0, color));
        painter.text(
            label,
            Align2::CENTER_CENTER,
            axis.letter(),
            FontId::proportional(11.0),
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

fn shade(rgb: [u8; 3], facing: f64) -> Color32 {
    let brightness = 0.75 + 0.35 * facing.clamp(0.0, 1.0);
    let [red, green, blue] = rgb.map(|channel| (f64::from(channel) * brightness).min(255.0) as u8);
    Color32::from_rgb(red, green, blue)
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
}
