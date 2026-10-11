#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Finish {
    Matte,
    Satin,
    Glossy,
    Rubber,
    PowderCoat,
    Anodized,
    BrushedMetal,
    PolishedMetal,
}

const METALS: [&str; 19] = [
    "steel",
    "stainless",
    "aluminium",
    "aluminum",
    "chrome",
    "chromium",
    "brass",
    "bronze",
    "copper",
    "metal",
    "metallic",
    "iron",
    "titanium",
    "zinc",
    "nickel",
    "gold",
    "silver",
    "galvanized",
    "galvanised",
];
const POLISHED: [&str; 7] = [
    "polished", "chrome", "chromium", "mirror", "glossy", "gloss", "shiny",
];
const ANODIZED: [&str; 2] = ["anodized", "anodised"];
const POWDER_COATS: [&str; 2] = ["powdercoat", "powdercoated"];
const RUBBERS: [&str; 5] = ["rubber", "silicone", "neoprene", "elastomer", "tpu"];
const GLOSSY: [&str; 8] = [
    "glossy",
    "gloss",
    "shiny",
    "polished",
    "glass",
    "lacquer",
    "enamel",
    "varnished",
];
const SATIN: [&str; 3] = ["satin", "semigloss", "eggshell"];
const MATTE: [&str; 6] = ["matte", "matt", "flat", "textured", "rough", "sandblasted"];
const PLASTICS: [&str; 10] = [
    "plastic",
    "abs",
    "pla",
    "petg",
    "nylon",
    "polycarbonate",
    "acrylic",
    "pom",
    "delrin",
    "polypropylene",
];

pub(crate) fn appearance_finish(name: &str) -> Option<Finish> {
    let words: Vec<String> = name
        .split(|letter: char| !letter.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let has = |list: &[&str]| words.iter().any(|word| list.contains(&word.as_str()));
    let powder_coat = has(&POWDER_COATS)
        || words.windows(2).any(
            |pair| matches!(pair, [powder, coat] if powder == "powder" && coat.starts_with("coat")),
        );
    if has(&ANODIZED) {
        Some(Finish::Anodized)
    } else if powder_coat {
        Some(Finish::PowderCoat)
    } else if has(&RUBBERS) {
        Some(Finish::Rubber)
    } else if has(&METALS) {
        Some(if has(&POLISHED) {
            Finish::PolishedMetal
        } else {
            Finish::BrushedMetal
        })
    } else if has(&GLOSSY) {
        Some(Finish::Glossy)
    } else if has(&SATIN) {
        Some(Finish::Satin)
    } else if has(&MATTE) {
        Some(Finish::Matte)
    } else if has(&PLASTICS) {
        Some(Finish::Satin)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Finish, appearance_finish};

    #[test]
    fn appearance_names_from_fusion_and_others_give_their_finish() {
        let named = [
            ("Steel - Satin", Some(Finish::BrushedMetal)),
            ("Stainless Steel - Polished", Some(Finish::PolishedMetal)),
            ("Aluminum - Anodized Glossy (Blue)", Some(Finish::Anodized)),
            ("Aluminium - Brushed", Some(Finish::BrushedMetal)),
            ("Chrome", Some(Finish::PolishedMetal)),
            ("Brass - Polished", Some(Finish::PolishedMetal)),
            ("Copper", Some(Finish::BrushedMetal)),
            ("Plastic - Matte (Black)", Some(Finish::Matte)),
            ("Plastic - Glossy (Red)", Some(Finish::Glossy)),
            ("ABS (White)", Some(Finish::Satin)),
            ("Powder Coat (Red)", Some(Finish::PowderCoat)),
            ("PEI Magnetic powder coated bed", Some(Finish::PowderCoat)),
            ("Rubber - Soft", Some(Finish::Rubber)),
            ("Paint - Enamel Glossy (Yellow)", Some(Finish::Glossy)),
            ("Satin finish", Some(Finish::Satin)),
            ("Glass (Smoked)", Some(Finish::Glossy)),
            ("Opaque(63,63,63)", None),
            ("Default", None),
            ("", None),
        ];

        for (name, finish) in named {
            assert_eq!(appearance_finish(name), finish, "{name}");
        }
    }

    #[test]
    fn a_word_only_counts_whole_so_a_name_containing_one_inside_another_gives_none() {
        assert_eq!(appearance_finish("Steelblue"), None);
        assert_eq!(appearance_finish("Platinum grey"), None);
        assert_eq!(appearance_finish("Tinted"), None);
    }
}
