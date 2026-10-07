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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShapeMode {
    Rectangle(RectangleMode),
    Circle(CircleMode),
    Polygon(PolygonMode),
    Slot(SlotMode),
}

impl ShapeMode {
    pub const ALL: [Self; 12] = [
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
        }
    }

    pub fn title(self) -> String {
        format!(
            "Draw {} {}",
            self.tool().label().to_lowercase(),
            self.label().to_lowercase()
        )
    }

    pub fn named(self) -> String {
        format!("{} {}", self.tool().label(), self.label().to_lowercase())
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
        }
    }

    pub fn hint(self, keys: Option<&str>) -> String {
        let named = self.named();
        match keys {
            Some(keys) => format!("{named}   {keys}: {}", self.next().label().to_lowercase()),
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
}

impl ShapeModes {
    pub fn of(&self, tool: Tool) -> Option<ShapeMode> {
        match tool {
            Tool::Rectangle => Some(ShapeMode::Rectangle(self.rectangle)),
            Tool::Circle => Some(ShapeMode::Circle(self.circle)),
            Tool::Polygon => Some(ShapeMode::Polygon(self.polygon)),
            Tool::Slot => Some(ShapeMode::Slot(self.slot)),
            Tool::Select
            | Tool::Point
            | Tool::Line
            | Tool::Arc
            | Tool::ThreePointArc
            | Tool::TangentArc
            | Tool::Spline
            | Tool::Trim
            | Tool::Extend
            | Tool::Offset
            | Tool::Mirror
            | Tool::Fillet
            | Tool::Project => None,
        }
    }

    pub fn set(&mut self, mode: ShapeMode) {
        match mode {
            ShapeMode::Rectangle(mode) => self.rectangle = mode,
            ShapeMode::Circle(mode) => self.circle = mode,
            ShapeMode::Polygon(mode) => self.polygon = mode,
            ShapeMode::Slot(mode) => self.slot = mode,
        }
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

    #[test]
    fn every_way_of_drawing_a_shape_is_listed() {
        let listed: Vec<ShapeMode> = RectangleMode::ALL
            .into_iter()
            .map(ShapeMode::Rectangle)
            .chain(CircleMode::ALL.into_iter().map(ShapeMode::Circle))
            .chain(PolygonMode::ALL.into_iter().map(ShapeMode::Polygon))
            .chain(SlotMode::ALL.into_iter().map(ShapeMode::Slot))
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
}
