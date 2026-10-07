use crate::variants::all_variants;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DisplayStyle {
    #[default]
    ShadedWithEdges,
    Shaded,
    Wireframe,
    HiddenLine,
    XRay,
}

all_variants!(DisplayStyle: ShadedWithEdges, Shaded, Wireframe, HiddenLine, XRay);

impl DisplayStyle {
    pub fn id(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "view.style_shaded_with_edges",
            Self::Shaded => "view.style_shaded",
            Self::Wireframe => "view.style_wireframe",
            Self::HiddenLine => "view.style_hidden_line",
            Self::XRay => "view.style_xray",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "Shaded with edges",
            Self::Shaded => "Shaded without edges",
            Self::Wireframe => "Wireframe",
            Self::HiddenLine => "Hidden lines removed",
            Self::XRay => "X-ray",
        }
    }

    pub fn shows_faces(self) -> bool {
        !matches!(self, Self::Wireframe)
    }

    pub fn is_translucent(self) -> bool {
        matches!(self, Self::XRay)
    }

    pub fn is_drawing(self) -> bool {
        matches!(self, Self::HiddenLine)
    }

    pub fn shows_edges(self) -> bool {
        !matches!(self, Self::Shaded)
    }
}
