use crate::variants::all_variants;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DisplayStyle {
    #[default]
    ShadedWithEdges,
    Shaded,
    Wireframe,
    XRay,
}

all_variants!(DisplayStyle: ShadedWithEdges, Shaded, Wireframe, XRay);

impl DisplayStyle {
    pub fn id(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "view.style_shaded_with_edges",
            Self::Shaded => "view.style_shaded",
            Self::Wireframe => "view.style_wireframe",
            Self::XRay => "view.style_xray",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ShadedWithEdges => "Shaded with edges",
            Self::Shaded => "Shaded without edges",
            Self::Wireframe => "Wireframe",
            Self::XRay => "X-ray",
        }
    }

    pub fn shows_faces(self) -> bool {
        !matches!(self, Self::Wireframe)
    }

    pub fn is_translucent(self) -> bool {
        matches!(self, Self::XRay)
    }

    pub fn shows_edges(self) -> bool {
        !matches!(self, Self::Shaded)
    }
}
