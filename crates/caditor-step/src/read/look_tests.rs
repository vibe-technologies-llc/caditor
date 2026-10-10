use std::time::SystemTime;

use crate::{
    fixtures,
    read::{FaceLook, StepModel, StepSolid, read_step},
    write::{StepBody, write_step},
};

const RED: [u8; 3] = [255, 0, 0];
const GREEN: [u8; 3] = [0, 255, 0];
const BLUE: [u8; 3] = [0, 0, 255];

struct Body {
    name: &'static str,
    colour: Option<[u8; 3]>,
    opacity: Option<u8>,
    faces: Vec<FaceLook>,
}

fn look(face: usize, colour: Option<[u8; 3]>, opacity: Option<u8>) -> FaceLook {
    FaceLook {
        face,
        colour,
        opacity,
    }
}

fn bodies(faces: usize) -> Vec<Body> {
    vec![
        Body {
            name: "Tinted",
            colour: Some(RED),
            opacity: Some(50),
            faces: vec![
                look(0, Some(GREEN), None),
                look(1, None, Some(100)),
                look(2, Some(BLUE), Some(25)),
                look(3, None, Some(75)),
            ],
        },
        Body {
            name: "Bare",
            colour: None,
            opacity: None,
            faces: vec![look(0, Some(GREEN), None), look(1, None, Some(50))],
        },
        Body {
            name: "Painted",
            colour: None,
            opacity: None,
            faces: (1..faces)
                .map(|face| look(face, Some(GREEN), None))
                .collect(),
        },
        Body {
            name: "Coloured",
            colour: Some(BLUE),
            opacity: None,
            faces: Vec::new(),
        },
        Body {
            name: "Clear",
            colour: None,
            opacity: Some(40),
            faces: Vec::new(),
        },
        Body {
            name: "Plain",
            colour: None,
            opacity: None,
            faces: Vec::new(),
        },
    ]
}

fn written(bodies: &[Body], solid: &caditor_kernel::Solid) -> String {
    let written: Vec<StepBody<'_>> = bodies
        .iter()
        .map(|body| StepBody {
            name: body.name,
            solid,
            colour: body.colour,
            opacity: body.opacity,
            layer: None,
            threads: &[],
            faces: &body.faces,
        })
        .collect();
    write_step(&written, "model", SystemTime::UNIX_EPOCH).unwrap()
}

fn assert_same_look(body: &Body, read: &StepSolid) {
    assert_eq!(
        (read.colour, read.opacity, &read.faces),
        (body.colour, body.opacity, &body.faces),
        "{}",
        body.name
    );
}

#[test]
fn a_body_and_a_mix_of_faces_come_back_with_exactly_their_colours_and_opacities() {
    let plate = fixtures::plate_with_hole();

    for body in bodies(plate.faces().count()) {
        let text = written(std::slice::from_ref(&body), &plate);
        let model = read_step(&text).unwrap();

        assert!(model.notes.is_empty(), "{}: {:?}", body.name, model.notes);
        let [read] = model.solids.as_slice() else {
            panic!("{}: expected one solid", body.name);
        };
        assert_same_look(&body, read);
    }
}

#[test]
fn an_assembly_comes_back_with_exactly_the_colours_and_opacities_of_its_parts() {
    let plate = fixtures::plate_with_hole();
    let bodies = bodies(plate.faces().count());
    let text = written(&bodies, &plate);

    let model = read_step(&text).unwrap();

    assert!(text.contains("NEXT_ASSEMBLY_USAGE_OCCURRENCE"));
    assert!(text.contains("NULL_STYLE(.NULL.)"));
    assert!(model.notes.is_empty(), "{:?}", model.notes);
    assert_eq!(model.solids.len(), bodies.len());
    for body in &bodies {
        let read = model
            .solids
            .iter()
            .find(|solid| solid.name == body.name)
            .unwrap();
        assert_same_look(body, read);
    }
}

fn inserted(text: &str, extra: &str) -> String {
    let mut styled = text.to_owned();
    let end = styled.rfind("ENDSEC;").unwrap();
    styled.insert_str(end, extra);
    styled
}

fn id_of<'t>(text: &'t str, start: &str) -> &'t str {
    text.lines()
        .find_map(|line| {
            let (id, rest) = line.split_once('=')?;
            rest.starts_with(start).then_some(id)
        })
        .unwrap_or_else(|| panic!("no line starting {start}"))
}

fn colour_style(id: u64, colour: &str) -> String {
    format!(
        "#{id}=PRESENTATION_STYLE_ASSIGNMENT((#{}));\n\
         #{}=SURFACE_STYLE_USAGE(.BOTH.,#{});\n\
         #{}=SURFACE_SIDE_STYLE('',(#{}));\n\
         #{}=SURFACE_STYLE_FILL_AREA(#{});\n\
         #{}=FILL_AREA_STYLE('',(#{}));\n\
         #{}=FILL_AREA_STYLE_COLOUR('',#{});\n\
         #{}=DRAUGHTING_PRE_DEFINED_COLOUR('{colour}');\n",
        id + 1,
        id + 1,
        id + 2,
        id + 2,
        id + 3,
        id + 3,
        id + 4,
        id + 4,
        id + 5,
        id + 5,
        id + 6,
        id + 6,
    )
}

fn named<'m>(model: &'m StepModel, name: &str) -> &'m StepSolid {
    model
        .solids
        .iter()
        .find(|solid| solid.name == name)
        .unwrap_or_else(|| {
            let names: Vec<&str> = model
                .solids
                .iter()
                .map(|solid| solid.name.as_str())
                .collect();
            panic!("no solid named {name} among {names:?}")
        })
}

#[test]
fn styles_on_one_copy_in_an_assembly_colour_that_copy_alone() {
    let plate = fixtures::plate_with_hole();
    let plain = |name| Body {
        name,
        colour: None,
        opacity: None,
        faces: Vec::new(),
    };
    let text = written(&[plain("Left"), plain("Right")], &plate);
    let root = id_of(&text, "SHAPE_REPRESENTATION(''");
    let context = text
        .lines()
        .find(|line| line.starts_with(&format!("{root}=")))
        .and_then(|line| line.rsplit_once(',')?.1.strip_suffix(");"))
        .unwrap();
    let left = id_of(&text, "ADVANCED_BREP_SHAPE_REPRESENTATION('Left'");
    let right = id_of(&text, "ADVANCED_BREP_SHAPE_REPRESENTATION('Right'");
    let left_solid = id_of(&text, "MANIFOLD_SOLID_BREP('Left'");
    let right_solid = id_of(&text, "MANIFOLD_SOLID_BREP('Right'");
    let left_placed = id_of(
        &text,
        &format!("(REPRESENTATION_RELATIONSHIP('','',{left},"),
    );
    let identity = id_of(&text, "ITEM_DEFINED_TRANSFORMATION(");
    let origin = id_of(&text, "AXIS2_PLACEMENT_3D(");
    let extra = format!(
        "{}{}{}{}\
         #950001=(REPRESENTATION_RELATIONSHIP('','',{left},{root})\
         REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION({identity})\
         SHAPE_REPRESENTATION_RELATIONSHIP());\n\
         #950010=STYLED_ITEM('',(#960000),{left_solid});\n\
         #950011=CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM('',(#961000),{left_solid},#950010,\
         (#950001));\n\
         #950012=PRESENTATION_STYLE_BY_CONTEXT((#963001),{left_placed});\n\
         #950013=STYLED_ITEM('',(#950012),{left_solid});\n\
         #950014=CONTEXT_DEPENDENT_OVER_RIDING_STYLED_ITEM('',(#961000),{right_solid},#950010,\
         (#950001));\n\
         #950020=REPRESENTATION_MAP({origin},{right});\n\
         #950021=MAPPED_ITEM('',#950020,{origin});\n\
         #950022=SHAPE_REPRESENTATION('',(#950021),{context});\n\
         #950023=STYLED_ITEM('',(#962000),#950021);\n",
        colour_style(960_000, "blue"),
        colour_style(961_000, "red"),
        colour_style(962_000, "green"),
        colour_style(963_000, "yellow"),
    );

    let model = read_step(&inserted(&text, &extra)).unwrap();

    assert_eq!(named(&model, "Left").colour, Some([255, 255, 0]));
    assert_eq!(named(&model, "Left 2").colour, Some(RED));
    assert_eq!(named(&model, "Right").colour, None);
    assert_eq!(named(&model, "Right 2").colour, Some(GREEN));
    assert_eq!(
        model.notes,
        [
            "Some styling in the file could not be understood, so what it styles keeps the look it \
          has without it: #950014, a style for one copy in the assembly that matches none of \
          them."
        ]
    );
}

fn plate_styled(extra: impl Fn(&str) -> String) -> StepModel {
    let plate = fixtures::plate_with_hole();
    let text = written(
        &[Body {
            name: "Plate",
            colour: None,
            opacity: None,
            faces: Vec::new(),
        }],
        &plate,
    );
    let solid = id_of(&text, "MANIFOLD_SOLID_BREP(").to_owned();
    read_step(&inserted(&text, &extra(&solid))).unwrap()
}

fn usage(id: u64, side: &str, elements: &str) -> String {
    format!(
        "#{id}=SURFACE_STYLE_USAGE(.{side}.,#{});\n#{}=SURFACE_SIDE_STYLE('',({elements}));\n",
        id + 1,
        id + 1
    )
}

#[test]
fn every_form_exporters_use_for_a_coloured_or_see_through_body_is_read() {
    let styled_by = |entities: &str, styles: &str| {
        let entities = entities.to_owned();
        let styles = styles.to_owned();
        let model = plate_styled(move |solid| {
            format!(
                "{entities}#970000=PRESENTATION_STYLE_ASSIGNMENT(({styles}));\n\
                 #970001=STYLED_ITEM('color',(#970000),{solid});\n"
            )
        });
        assert!(model.notes.is_empty(), "{:?}", model.notes);
        (model.solids[0].colour, model.solids[0].opacity)
    };
    let fill = |id: u64, colour: &str| {
        format!(
            "#{id}=SURFACE_STYLE_FILL_AREA(#{});\n#{}=FILL_AREA_STYLE('',(#{}));\n\
             #{}=FILL_AREA_STYLE_COLOUR('',{colour});\n",
            id + 1,
            id + 1,
            id + 2,
            id + 2
        )
    };
    let red = "#980000=COLOUR_RGB('',1.,0.,0.);\n";
    let green = "#980001=COLOUR_RGB('',0.,1.,0.);\n";

    let named = [
        fill(971_000, "#980002"),
        "#980002=DRAUGHTING_PRE_DEFINED_COLOUR('Cyan');\n".to_owned(),
    ]
    .concat();
    assert_eq!(
        styled_by(
            &[named, usage(972_000, "BOTH", "#971000")].concat(),
            "#972000"
        ),
        (Some([0, 255, 255]), None)
    );

    let rendered = format!(
        "{red}#971000=SURFACE_STYLE_RENDERING(.CONSTANT_SHADING.,#980000);\n{}",
        usage(972_000, "BOTH", "#971000")
    );
    assert_eq!(styled_by(&rendered, "#972000"), (Some(RED), None));

    let colourless = format!(
        "#971000=SURFACE_STYLE_TRANSPARENT(0.3);\n\
         #971001=SURFACE_STYLE_RENDERING_WITH_PROPERTIES(.NORMAL_SHADING.,$,(#971000));\n{}",
        usage(972_000, "BOTH", "#971001")
    );
    assert_eq!(styled_by(&colourless, "#972000"), (None, Some(70)));

    let bytes = [
        fill(971_000, "#980003"),
        "#980003=COLOUR_RGB('',255.,128.,0.);\n".to_owned(),
        usage(972_000, "BOTH", "#971000"),
    ]
    .concat();
    assert_eq!(styled_by(&bytes, "#972000"), (Some([255, 128, 0]), None));

    let sides = [
        red.to_owned(),
        green.to_owned(),
        "#973000=CURVE_STYLE('',$,$,#980001);\n".to_owned(),
        fill(971_000, "#980001"),
        "#971010=SURFACE_STYLE_TRANSPARENT(0.9);\n".to_owned(),
        usage(972_000, "NEGATIVE", "#971000,#971010"),
        fill(971_100, "#980000"),
        "#971110=SURFACE_STYLE_TRANSPARENT(0.5);\n\
         #971111=SURFACE_STYLE_RENDERING_WITH_PROPERTIES(.NORMAL_SHADING.,#980000,\
         (#971112,#971110));\n\
         #971112=SURFACE_STYLE_REFLECTANCE_AMBIENT(0.8);\n"
            .to_owned(),
        usage(972_100, "POSITIVE", "#971100,#971111"),
    ]
    .concat();
    assert_eq!(
        styled_by(&sides, "#973000,#972000,#972100"),
        (Some(RED), Some(50))
    );
    assert_eq!(
        styled_by(&sides, "#973000,#972000"),
        (Some(GREEN), Some(10))
    );

    let complex = plate_styled(|solid| {
        [
            colour_style(960_000, "magenta"),
            format!("#970001=(REPRESENTATION_ITEM('')STYLED_ITEM((#960000),{solid}));\n"),
        ]
        .concat()
    });
    assert_eq!(complex.solids[0].colour, Some([255, 0, 255]));

    let overridden = plate_styled(|solid| {
        [
            colour_style(960_000, "blue"),
            colour_style(961_000, "red"),
            format!(
                "#970001=OVER_RIDING_STYLED_ITEM('',(#961000),{solid},#970002);\n\
                 #970002=STYLED_ITEM('',(#960000),{solid});\n"
            ),
        ]
        .concat()
    });
    assert_eq!(overridden.solids[0].colour, Some(RED));

    let apart = plate_styled(|solid| {
        [
            colour_style(960_000, "blue"),
            "#961000=PRESENTATION_STYLE_ASSIGNMENT((#961001));\n".to_owned(),
            usage(961_001, "BOTH", "#961003"),
            "#961003=SURFACE_STYLE_TRANSPARENT(0.25);\n".to_owned(),
            format!(
                "#970001=STYLED_ITEM('',(#960000),{solid});\n\
                 #970002=STYLED_ITEM('',(#961000),{solid});\n"
            ),
        ]
        .concat()
    });
    assert_eq!(
        (apart.solids[0].colour, apart.solids[0].opacity),
        (Some(BLUE), Some(75))
    );
}

#[test]
fn a_colour_named_after_a_clear_appearance_is_see_through_unless_a_transparency_is_stated() {
    let named = |name: &str, transparency: Option<&str>| {
        let name = name.to_owned();
        let (elements, transparent) = match transparency {
            Some(value) => (
                "#971000,#971010".to_owned(),
                format!("#971010=SURFACE_STYLE_TRANSPARENT({value});\n"),
            ),
            None => ("#971000".to_owned(), String::new()),
        };
        let model = plate_styled(move |solid| {
            format!(
                "#980000=COLOUR_RGB('{name}',0.96,0.96,0.95);\n\
                 #971000=SURFACE_STYLE_FILL_AREA(#971001);\n\
                 #971001=FILL_AREA_STYLE('{name}',(#971002));\n\
                 #971002=FILL_AREA_STYLE_COLOUR('{name}',#980000);\n\
                 {transparent}{}#970000=PRESENTATION_STYLE_ASSIGNMENT((#972000));\n\
                 #970001=STYLED_ITEM('color',(#970000),{solid});\n",
                usage(972_000, "POSITIVE", &elements)
            )
        });
        assert!(model.notes.is_empty(), "{:?}", model.notes);
        (model.solids[0].colour, model.solids[0].opacity)
    };
    let pale = Some([245, 245, 242]);

    assert_eq!(named("Acrylic (Clear)", None), (pale, Some(25)));
    assert_eq!(named("Glass - Window", None), (pale, Some(25)));
    assert_eq!(named("Glass (Smoked)", None), (pale, Some(50)));
    assert_eq!(
        named("Plastic - Translucent Glossy (Gray)", None),
        (pale, Some(50))
    );
    assert_eq!(named("Opaque(245,245,242)", None), (pale, None));
    assert_eq!(named("Paint - Clear Coat", None), (pale, None));
    assert_eq!(named("Steel - Satin", None), (pale, None));
    assert_eq!(named("Unclear", None), (pale, None));
    assert_eq!(named("Acrylic (Clear)", Some("0.1")), (pale, Some(90)));
}

#[test]
fn styling_that_cannot_be_understood_is_named_and_what_it_styles_keeps_its_look() {
    let model = plate_styled(|solid| {
        [
            "#960000=PRESENTATION_STYLE_ASSIGNMENT((#960001,#960010));\n".to_owned(),
            usage(960_001, "BOTH", "#960003,#960006"),
            "#960003=SURFACE_STYLE_FILL_AREA(#960004);\n\
             #960004=FILL_AREA_STYLE('',(#960005,#960007));\n\
             #960005=FILL_AREA_STYLE_COLOUR('',#960008);\n\
             #960006=SURFACE_STYLE_TRANSPARENT(0.5);\n\
             #960007=FILL_AREA_STYLE_HATCHING('',#960009,$,$,$,$,0.);\n\
             #960008=DRAUGHTING_PRE_DEFINED_COLOUR('purple');\n\
             #960010=EXTERNALLY_DEFINED_STYLE(#960011,'gloss');\n"
                .to_owned(),
            format!("#970001=STYLED_ITEM('',(#960000),{solid});\n"),
            "#970100=CARTESIAN_POINT('',(0.,0.,0.));\n\
             #970101=STYLED_ITEM('',(#970102),#970100);\n\
             #970102=PRESENTATION_STYLE_ASSIGNMENT((#970103));\n"
                .to_owned(),
            usage(970_103, "BOTH", "#970105"),
            "#970105=SURFACE_STYLE_RENDERING(.NORMAL_SHADING.,#970106);\n\
             #970106=DRAUGHTING_PRE_DEFINED_COLOUR('mauve');\n"
                .to_owned(),
        ]
        .concat()
    });

    assert_eq!(
        (model.solids[0].colour, model.solids[0].opacity),
        (None, Some(50))
    );
    assert_eq!(
        model.notes,
        [
            "Some styling in the file could not be understood, so what it styles keeps the look it \
          has without it: #960007, a hatched or tiled fill; #960008, a colour named in a way \
          caditor does not know and #960010, a kind of style caditor does not know."
        ]
    );
}

#[test]
fn faces_of_a_body_imported_as_facets_keep_their_colours() {
    let cube = crate::read::tests::faceted_cube("FACETED_BREP('cube',#40)", false)
        .replace("(10.0,10.0,10.0)", "(10.0,10.0,10.0005)");
    let declared =
        crate::read::tests::with_precision(&cube, "FACETED_BREP_SHAPE_REPRESENTATION", 41, "1.E-3");
    let styled = inserted(
        &declared,
        &[
            colour_style(960_000, "red"),
            "#970001=STYLED_ITEM('',(#960000),#30);\n".to_owned(),
        ]
        .concat(),
    );

    let model = read_step(&styled).unwrap();

    let solid = &model.solids[0];
    assert!(
        model
            .notes
            .iter()
            .any(|note| note.contains("imported as flat facets")),
        "{:?}",
        model.notes
    );
    assert_eq!(solid.colour, None);
    assert!(!solid.faces.is_empty());
    for look in &solid.faces {
        assert_eq!((look.colour, look.opacity), (Some(RED), None));
        let (_, face) = solid.solid.faces().nth(look.face).unwrap();
        let caditor_kernel::Surface::Plane(plane) = face.surface() else {
            panic!("a faceted face is flat");
        };
        assert!(plane.frame().origin().z.abs() < 1e-6);
        assert!(plane.frame().normal().z.abs() > 0.999);
    }
}
