use crate::components::{Component, Inventory};

const INTRODUCTION: &str = "\
caditor is licensed under the GNU Affero General Public License, version 3 only; the file LICENSE
beside this one is its text. caditor contains the fonts and the Rust libraries listed below, each
under its own licence, and the texts of those licences follow the lists.

A library that ships no licence file of its own is given the standard text of the licences its
manifest names, with its authors as the copyright holders.";

const RULE: &str = "==============================================================================";

struct Block<'a> {
    text: &'a str,
    users: Vec<&'a str>,
}

pub fn render(inventory: &Inventory) -> String {
    let mut output = String::new();
    output.push_str("caditor third-party notices\n===========================\n\n");
    output.push_str(INTRODUCTION);
    output.push_str("\n\n");

    list(&mut output, "Fonts and icons", &inventory.assets);
    list(&mut output, "Rust libraries", &inventory.crates);

    output.push_str("Licence texts\n-------------\n");
    for block in blocks(inventory) {
        output.push('\n');
        output.push_str(RULE);
        output.push_str("\nApplies to:\n");
        for user in block.users {
            output.push_str("  - ");
            output.push_str(user);
            output.push('\n');
        }
        output.push_str(RULE);
        output.push_str("\n\n");
        output.push_str(block.text);
        output.push('\n');
    }
    output
}

fn list(output: &mut String, title: &str, components: &[Component]) {
    output.push_str(title);
    output.push('\n');
    output.push_str(&"-".repeat(title.len()));
    output.push('\n');
    for component in components {
        let license = if component.license.is_empty() {
            "licence not stated"
        } else {
            &component.license
        };
        output.push_str(&format!("{} ({license})\n", component.name));
    }
    output.push('\n');
}

fn blocks(inventory: &Inventory) -> Vec<Block<'_>> {
    let mut blocks: Vec<Block<'_>> = Vec::new();
    for component in inventory.assets.iter().chain(&inventory.crates) {
        for text in &component.texts {
            match blocks.iter_mut().find(|block| block.text == text) {
                Some(block) => block.users.push(&component.name),
                None => blocks.push(Block {
                    text,
                    users: vec![&component.name],
                }),
            }
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(name: &str, license: &str, texts: &[&str]) -> Component {
        Component {
            name: name.to_owned(),
            license: license.to_owned(),
            texts: texts.iter().map(|text| (*text).to_owned()).collect(),
        }
    }

    fn inventory() -> Inventory {
        Inventory {
            assets: vec![component("Font", "OFL-1.1", &["OFL text"])],
            crates: vec![
                component("a 1.0.0", "MIT", &["MIT text"]),
                component("b 2.0.0", "MIT OR Apache-2.0", &["MIT text", "Apache text"]),
                component("c 3.0.0", "", &["OFL text"]),
            ],
        }
    }

    #[test]
    fn each_text_is_written_once_with_every_component_that_carries_it() {
        let notices = render(&inventory());

        assert_eq!(notices.matches("MIT text").count(), 1);
        assert_eq!(notices.matches("OFL text").count(), 1);
        assert!(notices.contains("Applies to:\n  - Font\n  - c 3.0.0\n"));
        assert!(notices.contains("Applies to:\n  - a 1.0.0\n  - b 2.0.0\n"));
        assert!(notices.contains("Applies to:\n  - b 2.0.0\n"));
    }

    #[test]
    fn the_lists_name_every_component_with_its_licence() {
        let notices = render(&inventory());

        assert!(notices.contains("Fonts and icons\n---------------\nFont (OFL-1.1)\n"));
        assert!(notices.contains("b 2.0.0 (MIT OR Apache-2.0)\n"));
        assert!(notices.contains("c 3.0.0 (licence not stated)\n"));
    }

    #[test]
    fn rendering_twice_gives_the_same_bytes() {
        assert_eq!(render(&inventory()), render(&inventory()));
    }
}
