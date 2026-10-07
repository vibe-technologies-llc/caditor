pub const MAX_PROPERTY_CHARS: usize = 200;
pub const MAX_DESCRIPTION_CHARS: usize = 1000;
pub const MAX_MODEL_NOTES_CHARS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ModelProperty {
    Title,
    PartNumber,
    Revision,
    Author,
    Organisation,
    Description,
    Notes,
}

impl ModelProperty {
    pub const ALL: [Self; 7] = [
        Self::Title,
        Self::PartNumber,
        Self::Revision,
        Self::Author,
        Self::Organisation,
        Self::Description,
        Self::Notes,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::PartNumber => "Part number",
            Self::Revision => "Revision",
            Self::Author => "Author",
            Self::Organisation => "Organisation",
            Self::Description => "Description",
            Self::Notes => "Notes",
        }
    }

    pub fn in_sentence(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::PartNumber => "part number",
            Self::Revision => "revision",
            Self::Author => "author",
            Self::Organisation => "organisation",
            Self::Description => "description",
            Self::Notes => "notes",
        }
    }

    pub fn is_multiline(self) -> bool {
        self == Self::Notes
    }

    pub fn max_chars(self) -> usize {
        match self {
            Self::Title | Self::PartNumber | Self::Revision | Self::Author | Self::Organisation => {
                MAX_PROPERTY_CHARS
            }
            Self::Description => MAX_DESCRIPTION_CHARS,
            Self::Notes => MAX_MODEL_NOTES_CHARS,
        }
    }

    pub fn normalized(self, text: &str) -> String {
        let trimmed = text.trim();
        if self.is_multiline() {
            return trimmed.to_owned();
        }
        trimmed
            .split(['\r', '\n'])
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelProperties {
    pub title: String,
    pub part_number: String,
    pub revision: String,
    pub author: String,
    pub organisation: String,
    pub description: String,
    pub notes: String,
}

impl ModelProperties {
    pub fn get(&self, property: ModelProperty) -> &str {
        match property {
            ModelProperty::Title => &self.title,
            ModelProperty::PartNumber => &self.part_number,
            ModelProperty::Revision => &self.revision,
            ModelProperty::Author => &self.author,
            ModelProperty::Organisation => &self.organisation,
            ModelProperty::Description => &self.description,
            ModelProperty::Notes => &self.notes,
        }
    }

    pub fn get_mut(&mut self, property: ModelProperty) -> &mut String {
        match property {
            ModelProperty::Title => &mut self.title,
            ModelProperty::PartNumber => &mut self.part_number,
            ModelProperty::Revision => &mut self.revision,
            ModelProperty::Author => &mut self.author,
            ModelProperty::Organisation => &mut self.organisation,
            ModelProperty::Description => &mut self.description,
            ModelProperty::Notes => &mut self.notes,
        }
    }

    pub fn is_empty(&self) -> bool {
        ModelProperty::ALL
            .into_iter()
            .all(|property| self.get(property).is_empty())
    }

    #[must_use]
    pub fn normalized(mut self) -> Self {
        for property in ModelProperty::ALL {
            let field = self.get_mut(property);
            *field = property.normalized(field);
        }
        self
    }

    pub fn too_long(&self) -> Option<(ModelProperty, usize)> {
        ModelProperty::ALL.into_iter().find_map(|property| {
            let length = self.get(property).chars().count();
            (length > property.max_chars()).then_some((property, length))
        })
    }

    pub fn heap_size(&self) -> usize {
        ModelProperty::ALL
            .into_iter()
            .map(|property| self.get(property).len())
            .sum()
    }
}
