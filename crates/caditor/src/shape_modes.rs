use caditor_sketch::{Continuity, SplineKind};

use crate::editing::Tool;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RectangleMode {
    #[default]
    Corners,
    Center,
    ThreePoints,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CircleMode {
    #[default]
    Center,
    TwoPoints,
    ThreePoints,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PolygonMode {
    #[default]
    Corner,
    SideMiddle,
    Side,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotMode {
    #[default]
    Ends,
    Center,
    Arc,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SplineMode {
    #[default]
    Control,
    Fit,
    ClosedControl,
    ClosedFit,
}

impl SplineMode {
    pub fn kind(self, closed: bool) -> SplineKind {
        match self {
            Self::Control | Self::ClosedControl => SplineKind::Control {
                closed: closed || self.closes(),
            },
            Self::Fit | Self::ClosedFit => SplineKind::Fit {
                closed: closed || self.closes(),
            },
        }
    }

    pub fn closes(self) -> bool {
        matches!(self, Self::ClosedControl | Self::ClosedFit)
    }

    pub fn passes_its_points(self) -> bool {
        matches!(self, Self::Fit | Self::ClosedFit)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlendMode {
    #[default]
    Tangent,
    Curvature,
}

impl BlendMode {
    pub fn continuity(self) -> Continuity {
        match self {
            Self::Tangent => Continuity::Tangent,
            Self::Curvature => Continuity::Curvature,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShapeMode {
    Rectangle(RectangleMode),
    Circle(CircleMode),
    Polygon(PolygonMode),
    Slot(SlotMode),
    Spline(SplineMode),
    Blend(BlendMode),
}

impl ShapeMode {
    pub const ALL: [Self; 18] = [
        Self::Rectangle(RectangleMode::Corners),
        Self::Rectangle(RectangleMode::Center),
        Self::Rectangle(RectangleMode::ThreePoints),
        Self::Circle(CircleMode::Center),
        Self::Circle(CircleMode::TwoPoints),
        Self::Circle(CircleMode::ThreePoints),
        Self::Polygon(PolygonMode::Corner),
        Self::Polygon(PolygonMode::SideMiddle),
        Self::Polygon(PolygonMode::Side),
        Self::Slot(SlotMode::Ends),
        Self::Slot(SlotMode::Center),
        Self::Slot(SlotMode::Arc),
        Self::Spline(SplineMode::Control),
        Self::Spline(SplineMode::Fit),
        Self::Spline(SplineMode::ClosedControl),
        Self::Spline(SplineMode::ClosedFit),
        Self::Blend(BlendMode::Tangent),
        Self::Blend(BlendMode::Curvature),
    ];

    pub fn of_tool(tool: Tool) -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(move |mode| mode.tool() == tool)
    }

    pub fn tool(self) -> Tool {
        match self {
            Self::Rectangle(_) => Tool::Rectangle,
            Self::Circle(_) => Tool::Circle,
            Self::Polygon(_) => Tool::Polygon,
            Self::Slot(_) => Tool::Slot,
            Self::Spline(_) => Tool::Spline,
            Self::Blend(_) => Tool::BlendCurve,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Rectangle(mode) => Self::Rectangle(match mode {
                RectangleMode::Corners => RectangleMode::Center,
                RectangleMode::Center => RectangleMode::ThreePoints,
                RectangleMode::ThreePoints => RectangleMode::Corners,
            }),
            Self::Circle(mode) => Self::Circle(match mode {
                CircleMode::Center => CircleMode::TwoPoints,
                CircleMode::TwoPoints => CircleMode::ThreePoints,
                CircleMode::ThreePoints => CircleMode::Center,
            }),
            Self::Polygon(mode) => Self::Polygon(match mode {
                PolygonMode::Corner => PolygonMode::SideMiddle,
                PolygonMode::SideMiddle => PolygonMode::Side,
                PolygonMode::Side => PolygonMode::Corner,
            }),
            Self::Slot(mode) => Self::Slot(match mode {
                SlotMode::Ends => SlotMode::Center,
                SlotMode::Center => SlotMode::Arc,
                SlotMode::Arc => SlotMode::Ends,
            }),
            Self::Spline(mode) => Self::Spline(match mode {
                SplineMode::Control => SplineMode::Fit,
                SplineMode::Fit => SplineMode::ClosedControl,
                SplineMode::ClosedControl => SplineMode::ClosedFit,
                SplineMode::ClosedFit => SplineMode::Control,
            }),
            Self::Blend(mode) => Self::Blend(match mode {
                BlendMode::Tangent => BlendMode::Curvature,
                BlendMode::Curvature => BlendMode::Tangent,
            }),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rectangle(RectangleMode::Corners) => "From two corners",
            Self::Rectangle(RectangleMode::Center) => "From its centre",
            Self::Rectangle(RectangleMode::ThreePoints) => "From three points",
            Self::Circle(CircleMode::Center) => "From its centre",
            Self::Circle(CircleMode::TwoPoints) => "Through two points",
            Self::Circle(CircleMode::ThreePoints) => "Through three points",
            Self::Polygon(PolygonMode::Corner) => "From its centre and a corner",
            Self::Polygon(PolygonMode::SideMiddle) => "From its centre and a side's middle",
            Self::Polygon(PolygonMode::Side) => "From one side",
            Self::Slot(SlotMode::Ends) => "From the centres of its ends",
            Self::Slot(SlotMode::Center) => "From its centre",
            Self::Slot(SlotMode::Arc) => "Along an arc",
            Self::Spline(SplineMode::Control) => "By control points",
            Self::Spline(SplineMode::Fit) => "Through fit points",
            Self::Spline(SplineMode::ClosedControl) => "Closed, by control points",
            Self::Spline(SplineMode::ClosedFit) => "Closed, through fit points",
            Self::Blend(BlendMode::Tangent) => "Tangent (G1)",
            Self::Blend(BlendMode::Curvature) => "Curvature-continuous (G2)",
        }
    }

    pub fn title(self) -> String {
        match self {
            Self::Blend(_) => format!(
                "Draw a {} {}",
                decapitalized(self.label()),
                self.tool().label().to_lowercase()
            ),
            _ => format!(
                "Draw {} {}",
                self.tool().label().to_lowercase(),
                decapitalized(self.label())
            ),
        }
    }

    pub fn named(self) -> String {
        format!("{} {}", self.tool().label(), decapitalized(self.label()))
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Rectangle(RectangleMode::Corners) => "Draw a rectangle from two opposite corners",
            Self::Rectangle(RectangleMode::Center) => {
                "Draw a rectangle from its centre and a corner, kept centred on it"
            }
            Self::Rectangle(RectangleMode::ThreePoints) => {
                "Draw a rectangle at any angle: one side from corner to corner, then its width"
            }
            Self::Circle(CircleMode::Center) => "Draw a circle from its centre and a point on it",
            Self::Circle(CircleMode::TwoPoints) => {
                "Draw a circle through the two ends of a diameter"
            }
            Self::Circle(CircleMode::ThreePoints) => "Draw a circle through three points on it",
            Self::Polygon(PolygonMode::Corner) => {
                "Draw a regular polygon from its centre and a corner"
            }
            Self::Polygon(PolygonMode::SideMiddle) => {
                "Draw a regular polygon from its centre and the middle of a side, sized across \
                 its flats by a circle inside it"
            }
            Self::Polygon(PolygonMode::Side) => {
                "Draw a regular polygon from the two ends of one side, lying to the left of it"
            }
            Self::Slot(SlotMode::Ends) => {
                "Draw a slot from the centres of its round ends and its width"
            }
            Self::Slot(SlotMode::Center) => {
                "Draw a slot from its centre, the centre of one end and its width, kept \
                 centred on it"
            }
            Self::Slot(SlotMode::Arc) => {
                "Draw a curved slot along an arc from the arc's centre, the centres of its ends \
                 and its width"
            }
            Self::Spline(SplineMode::Control) => {
                "Draw a spline bending toward control points, open unless it ends on its first \
                 point"
            }
            Self::Spline(SplineMode::Fit) => {
                "Draw a spline passing through every point placed, each a point to constrain and \
                 dimension"
            }
            Self::Spline(SplineMode::ClosedControl) => {
                "Draw a smooth closed loop bending toward control points, closed on finishing"
            }
            Self::Spline(SplineMode::ClosedFit) => {
                "Draw a smooth closed loop passing through every point placed, closed on \
                 finishing"
            }
            Self::Blend(BlendMode::Tangent) => {
                "Join the ends of two curves with a spline leaving each along its direction (G1)"
            }
            Self::Blend(BlendMode::Curvature) => {
                "Join the ends of two curves with a spline leaving each along its direction and \
                 bending as tightly as it does there (G2)"
            }
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Rectangle(RectangleMode::Corners) => "sketch.rectangle.corners",
            Self::Rectangle(RectangleMode::Center) => "sketch.rectangle.center",
            Self::Rectangle(RectangleMode::ThreePoints) => "sketch.rectangle.three_points",
            Self::Circle(CircleMode::Center) => "sketch.circle.center",
            Self::Circle(CircleMode::TwoPoints) => "sketch.circle.two_points",
            Self::Circle(CircleMode::ThreePoints) => "sketch.circle.three_points",
            Self::Polygon(PolygonMode::Corner) => "sketch.polygon.corner",
            Self::Polygon(PolygonMode::SideMiddle) => "sketch.polygon.side_middle",
            Self::Polygon(PolygonMode::Side) => "sketch.polygon.side",
            Self::Slot(SlotMode::Ends) => "sketch.slot.ends",
            Self::Slot(SlotMode::Center) => "sketch.slot.center",
            Self::Slot(SlotMode::Arc) => "sketch.slot.arc",
            Self::Spline(SplineMode::Control) => "sketch.spline.control",
            Self::Spline(SplineMode::Fit) => "sketch.spline.fit",
            Self::Spline(SplineMode::ClosedControl) => "sketch.spline.closed_control",
            Self::Spline(SplineMode::ClosedFit) => "sketch.spline.closed_fit",
            Self::Blend(BlendMode::Tangent) => "sketch.blend_curve.tangent",
            Self::Blend(BlendMode::Curvature) => "sketch.blend_curve.curvature",
        }
    }

    pub fn hint(self, keys: Option<&str>) -> String {
        let named = self.named();
        match keys {
            Some(keys) => format!("{named}   {keys}: {}", decapitalized(self.next().label())),
            None => named,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShapeModes {
    rectangle: RectangleMode,
    circle: CircleMode,
    polygon: PolygonMode,
    slot: SlotMode,
    spline: SplineMode,
    blend: BlendMode,
}

impl ShapeModes {
    pub fn of(&self, tool: Tool) -> Option<ShapeMode> {
        match tool {
            Tool::Rectangle => Some(ShapeMode::Rectangle(self.rectangle)),
            Tool::Circle => Some(ShapeMode::Circle(self.circle)),
            Tool::Polygon => Some(ShapeMode::Polygon(self.polygon)),
            Tool::Slot => Some(ShapeMode::Slot(self.slot)),
            Tool::Spline => Some(ShapeMode::Spline(self.spline)),
            Tool::BlendCurve => Some(ShapeMode::Blend(self.blend)),
            Tool::Select
            | Tool::Point
            | Tool::Line
            | Tool::Arc
            | Tool::ThreePointArc
            | Tool::TangentArc
            | Tool::Ellipse
            | Tool::EllipticalArc
            | Tool::Trim
            | Tool::Extend
            | Tool::Offset
            | Tool::Mirror
            | Tool::RectangularPattern
            | Tool::CircularPattern
            | Tool::TangentCircle
            | Tool::Fillet
            | Tool::Chamfer
            | Tool::Project
            | Tool::Intersect
            | Tool::Dimension => None,
        }
    }

    pub fn blend(&self) -> Continuity {
        self.blend.continuity()
    }

    pub fn set(&mut self, mode: ShapeMode) {
        match mode {
            ShapeMode::Rectangle(mode) => self.rectangle = mode,
            ShapeMode::Circle(mode) => self.circle = mode,
            ShapeMode::Polygon(mode) => self.polygon = mode,
            ShapeMode::Slot(mode) => self.slot = mode,
            ShapeMode::Spline(mode) => self.spline = mode,
            ShapeMode::Blend(mode) => self.blend = mode,
        }
    }
}

fn decapitalized(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_lowercase().chain(characters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variants::all_variants;

    all_variants!(RectangleMode: Corners, Center, ThreePoints);
    all_variants!(CircleMode: Center, TwoPoints, ThreePoints);
    all_variants!(PolygonMode: Corner, SideMiddle, Side);
    all_variants!(SlotMode: Ends, Center, Arc);
    all_variants!(SplineMode: Control, Fit, ClosedControl, ClosedFit);
    all_variants!(BlendMode: Tangent, Curvature);

    #[test]
    fn every_way_of_drawing_a_shape_is_listed() {
        let listed: Vec<ShapeMode> = RectangleMode::ALL
            .into_iter()
            .map(ShapeMode::Rectangle)
            .chain(CircleMode::ALL.into_iter().map(ShapeMode::Circle))
            .chain(PolygonMode::ALL.into_iter().map(ShapeMode::Polygon))
            .chain(SlotMode::ALL.into_iter().map(ShapeMode::Slot))
            .chain(SplineMode::ALL.into_iter().map(ShapeMode::Spline))
            .chain(BlendMode::ALL.into_iter().map(ShapeMode::Blend))
            .collect();

        assert_eq!(listed, ShapeMode::ALL);
    }

    #[test]
    fn each_shape_cycles_through_its_own_modes_and_back() {
        for tool in [Tool::Rectangle, Tool::Circle, Tool::Polygon, Tool::Slot] {
            let modes: Vec<ShapeMode> = ShapeMode::of_tool(tool).collect();
            let first = ShapeModes::default().of(tool).unwrap();
            let cycled: Vec<ShapeMode> = std::iter::successors(Some(first), |mode| {
                Some(mode.next()).filter(|next| *next != first)
            })
            .collect();

            assert_eq!(modes.len(), 3);
            assert_eq!(cycled, modes);
            assert!(modes.iter().all(|mode| mode.tool() == tool));
        }
    }

    #[test]
    fn the_mode_set_last_is_the_one_remembered_for_its_shape_alone() {
        let mut modes = ShapeModes::default();

        modes.set(ShapeMode::Circle(CircleMode::ThreePoints));
        modes.set(ShapeMode::Slot(SlotMode::Arc));

        assert_eq!(
            modes.of(Tool::Circle),
            Some(ShapeMode::Circle(CircleMode::ThreePoints))
        );
        assert_eq!(modes.of(Tool::Slot), Some(ShapeMode::Slot(SlotMode::Arc)));
        assert_eq!(
            modes.of(Tool::Rectangle),
            Some(ShapeMode::Rectangle(RectangleMode::Corners))
        );
        assert_eq!(modes.of(Tool::Line), None);
    }

    #[test]
    fn a_mode_is_named_after_its_shape_and_its_hint_names_the_next() {
        let centre = ShapeMode::Rectangle(RectangleMode::Center);

        assert_eq!(centre.title(), "Draw rectangle from its centre");
        assert_eq!(centre.named(), "Rectangle from its centre");
        assert_eq!(
            centre.hint(Some("R")),
            "Rectangle from its centre   R: from three points"
        );
        assert_eq!(centre.hint(None), "Rectangle from its centre");
    }

    #[test]
    fn a_blend_switches_between_tangent_and_curvature_continuous() {
        let tangent = ShapeModes::default().of(Tool::BlendCurve).unwrap();

        assert_eq!(tangent, ShapeMode::Blend(BlendMode::Tangent));
        assert_eq!(tangent.next().next(), tangent);
        assert_eq!(tangent.title(), "Draw a tangent (G1) blend curve");
        assert_eq!(
            tangent.hint(Some("Alt+Shift+B")),
            "Blend curve tangent (G1)   Alt+Shift+B: curvature-continuous (G2)"
        );

        let mut modes = ShapeModes::default();
        modes.set(tangent.next());

        assert_eq!(modes.blend(), Continuity::Curvature);
    }
}
