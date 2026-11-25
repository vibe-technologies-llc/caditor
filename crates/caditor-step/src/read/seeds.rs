use std::{
    f64::consts::TAU,
    fmt::Write,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    StepBody, fixtures,
    read::{
        read_step,
        structure::{MAX_INSTANCES, MAX_WALKED_PLACEMENTS},
        tally,
    },
    write_step,
};

const WRITE_SEEDS: &str = "CADITOR_WRITE_FUZZ_SEEDS";
const STYLE_FAN_OUT: usize = 50;
const UNREACHABLE_STYLE_LEVELS: usize = 10;
const LAYERS: usize = 30;
const JOINED_REPRESENTATIONS: usize = 300;
const MAPPED_ITEMS: usize = 300;
const MAPPED_TARGETS: usize = 12;
const NESTED_PLACEMENTS: usize = 31;
const PRISM_SIDES: usize = 200;
const LIST_NESTING: usize = 60;
const REPLICA_CHAIN: usize = 8;
const WORK_PER_BYTE: usize = 1;
const PLACEMENTS_CUT_SHORT: &str = "The file places parts more than";
const REFUSED: &str = "refused-";

fn seeds_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds/step")
}

#[derive(Default)]
struct Data {
    text: String,
    last: u64,
}

impl Data {
    fn add(&mut self, entity: &str) -> u64 {
        self.last += 1;
        writeln!(self.text, "#{}={entity};", self.last).unwrap();
        self.last
    }

    fn exchange(&self) -> String {
        format!(
            "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION((''),'2;1');\n\
             FILE_NAME('seed','1970-01-01T00:00:00',(''),(''),'','','');\n\
             FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\nENDSEC;\nDATA;\n{}\
             ENDSEC;\nEND-ISO-10303-21;\n",
            self.text
        )
    }

    fn point(&mut self, [x, y, z]: [f64; 3]) -> u64 {
        self.add(&format!("CARTESIAN_POINT('',({x:.6},{y:.6},{z:.6}))"))
    }

    fn axis(&mut self, origin: [f64; 3]) -> u64 {
        let at = self.point(origin);
        let up = self.add("DIRECTION('',(0.,0.,1.))");
        let across = self.add("DIRECTION('',(1.,0.,0.))");

        self.add(&format!("AXIS2_PLACEMENT_3D('',#{at},#{up},#{across})"))
    }

    fn context(&mut self) -> u64 {
        let unit = self.add("(LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.))");
        let uncertainty = self.add(&format!(
            "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-6),#{unit},\
             'distance_accuracy_value','')"
        ));

        self.add(&format!(
            "(GEOMETRIC_REPRESENTATION_CONTEXT(3) \
             GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{uncertainty})) \
             GLOBAL_UNIT_ASSIGNED_CONTEXT((#{unit})) REPRESENTATION_CONTEXT('',''))"
        ))
    }

    fn product(&mut self, name: &str, representation: u64) {
        let application = self.add("APPLICATION_CONTEXT('automotive design')");
        let context = self.add(&format!("PRODUCT_CONTEXT('',#{application},'mechanical')"));
        let product = self.add(&format!("PRODUCT('{name}','{name}','',(#{context}))"));
        let formation = self.add(&format!("PRODUCT_DEFINITION_FORMATION('','',#{product})"));
        let definition_context = self.add(&format!(
            "PRODUCT_DEFINITION_CONTEXT('part definition',#{application},'design')"
        ));
        let definition = self.add(&format!(
            "PRODUCT_DEFINITION('design','',#{formation},#{definition_context})"
        ));
        let shape = self.add(&format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));

        self.add(&format!(
            "SHAPE_DEFINITION_REPRESENTATION(#{shape},#{representation})"
        ));
    }

    fn prism(&mut self, name: &str, sides: usize, context: u64) -> Prism {
        let ring = |data: &mut Self, height: f64| -> Vec<u64> {
            (0..sides)
                .map(|side| {
                    let angle = TAU * side as f64 / sides as f64;

                    data.point([10.0 * angle.cos(), 10.0 * angle.sin(), height])
                })
                .collect()
        };
        let bottom = ring(self, 0.0);
        let top = ring(self, 10.0);

        let mut loops: Vec<Vec<u64>> = vec![bottom.iter().rev().copied().collect(), top.clone()];

        for side in 0..sides {
            let next = (side + 1) % sides;

            loops.push(vec![bottom[side], bottom[next], top[next], top[side]]);
        }

        let faces: Vec<u64> = loops
            .iter()
            .map(|corners| {
                let polygon = self.add(&format!("POLY_LOOP('',({}))", references(corners)));
                let bound = self.add(&format!("FACE_OUTER_BOUND('',#{polygon},.T.)"));

                self.add(&format!("FACE('',(#{bound}))"))
            })
            .collect();
        let shell = self.add(&format!("CLOSED_SHELL('',({}))", references(&faces)));
        let brep = self.add(&format!("FACETED_BREP('{name}',#{shell})"));
        let representation = self.add(&format!(
            "FACETED_BREP_SHAPE_REPRESENTATION('{name}',(#{brep}),#{context})"
        ));

        self.product(name, representation);
        Prism {
            brep,
            faces,
            representation,
        }
    }

    fn placed(&mut self, child: u64, parent: u64, transformation: u64) -> u64 {
        self.add(&format!(
            "(REPRESENTATION_RELATIONSHIP('','',#{child},#{parent}) \
             REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{transformation}) \
             SHAPE_REPRESENTATION_RELATIONSHIP())"
        ))
    }

    fn moved(&mut self, from: u64, by: [f64; 3]) -> u64 {
        let to = self.axis(by);

        self.add(&format!("ITEM_DEFINED_TRANSFORMATION('','',#{from},#{to})"))
    }
}

struct Prism {
    brep: u64,
    faces: Vec<u64>,
    representation: u64,
}

fn references(ids: &[u64]) -> String {
    ids.iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn repeated(id: u64, times: usize) -> String {
    references(&vec![id; times])
}

fn styles_fanned() -> String {
    let mut data = Data::default();

    let context = data.context();
    let prism = data.prism("Styled", 6, context);

    let colour = data.add("COLOUR_RGB('Anodized aluminium',0.2,0.4,0.8)");
    let fill_colour = data.add(&format!("FILL_AREA_STYLE_COLOUR('',#{colour})"));
    let fill = data.add(&format!(
        "FILL_AREA_STYLE('',({}))",
        repeated(fill_colour, STYLE_FAN_OUT)
    ));
    let area = data.add(&format!("SURFACE_STYLE_FILL_AREA(#{fill})"));
    let side = data.add(&format!(
        "SURFACE_SIDE_STYLE('',({}))",
        repeated(area, STYLE_FAN_OUT)
    ));
    let usage = data.add(&format!("SURFACE_STYLE_USAGE(.BOTH.,#{side})"));

    let mut reached = usage;

    for _ in 0..3 {
        reached = data.add(&format!(
            "PRESENTATION_STYLE_ASSIGNMENT(({}))",
            repeated(reached, STYLE_FAN_OUT)
        ));
    }

    let mut unreachable = usage;

    for _ in 0..UNREACHABLE_STYLE_LEVELS {
        unreachable = data.add(&format!(
            "PRESENTATION_STYLE_ASSIGNMENT(({}))",
            repeated(unreachable, STYLE_FAN_OUT)
        ));
    }

    let mut styled = vec![data.add(&format!(
        "STYLED_ITEM('',(#{reached},#{unreachable}),#{})",
        prism.brep
    ))];

    for face in &prism.faces {
        styled.push(data.add(&format!(
            "STYLED_ITEM('',(#{unreachable},#{reached}),#{face})"
        )));
    }
    data.add(&format!(
        "MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',({}),#{context})",
        references(&styled)
    ));
    data.exchange()
}

fn assembly_layered() -> String {
    let mut data = Data::default();

    let context = data.context();
    let prism = data.prism("Layered", 4, context);
    let origin = data.axis([0.0; 3]);

    let mut layer = [
        prism.representation,
        data.add(&format!("SHAPE_REPRESENTATION('',(#{origin}),#{context})")),
    ];
    let moves: Vec<u64> = [
        [0.0, 0.0, 0.0],
        [20.0, 0.0, 0.0],
        [0.0, 20.0, 0.0],
        [0.0, 0.0, 20.0],
    ]
    .into_iter()
    .map(|by| data.moved(origin, by))
    .collect();

    for _ in 0..LAYERS {
        let parents =
            [0, 1].map(|_| data.add(&format!("SHAPE_REPRESENTATION('',(#{origin}),#{context})")));

        for (side, child) in layer.iter().enumerate() {
            for (index, parent) in parents.iter().enumerate() {
                data.placed(*child, *parent, moves[side * 2 + index]);
            }
        }
        layer = parents;
    }
    data.exchange()
}

fn assembly_mapped() -> String {
    let mut data = Data::default();

    let context = data.context();
    let prism = data.prism("Mapped", 5, context);
    let origin = data.axis([0.0; 3]);

    let mut members = vec![prism.representation];

    for _ in 1..JOINED_REPRESENTATIONS {
        members.push(data.add(&format!("SHAPE_REPRESENTATION('',(#{origin}),#{context})")));
    }
    for pair in members.windows(2) {
        data.add(&format!(
            "REPRESENTATION_RELATIONSHIP('','',#{},#{})",
            pair[0], pair[1]
        ));
    }

    let targets: Vec<u64> = (0..MAPPED_TARGETS)
        .map(|target| data.axis([25.0 * target as f64, 0.0, 0.0]))
        .collect();

    for item in 0..MAPPED_ITEMS {
        let map = data.add(&format!(
            "REPRESENTATION_MAP(#{origin},#{})",
            members[item % members.len()]
        ));
        let mapped = data.add(&format!(
            "MAPPED_ITEM('',#{map},#{})",
            targets[item % targets.len()]
        ));

        data.add(&format!("SHAPE_REPRESENTATION('',(#{mapped}),#{context})"));
    }
    data.exchange()
}

fn nested_list(depth: usize) -> String {
    format!("{}1.{}", "(".repeat(depth), ")".repeat(depth))
}

fn assembly_deep() -> String {
    let mut data = Data::default();

    let context = data.context();
    let prism = data.prism("Deep", PRISM_SIDES, context);
    let origin = data.axis([0.0; 3]);
    let step = data.moved(origin, [0.0, 0.0, 12.0]);

    let mut child = prism.representation;

    for _ in 0..NESTED_PLACEMENTS {
        let parent = data.add(&format!("SHAPE_REPRESENTATION('',(#{origin}),#{context})"));

        data.placed(child, parent, step);
        child = parent;
    }
    data.add(&format!(
        "DESCRIPTIVE_REPRESENTATION_ITEM('nested',{})",
        nested_list(LIST_NESTING)
    ));
    data.exchange()
}

fn replicas_deep() -> String {
    let plate = fixtures::plate_with_hole();
    let text = write_step(
        &[StepBody {
            name: "Replicated",
            solid: &plate,
            colour: None,
            opacity: None,
            layer: None,
            threads: &[],
            faces: &[],
        }],
        "Replicated",
        SystemTime::UNIX_EPOCH,
    )
    .unwrap();
    let last = text
        .lines()
        .filter_map(|line| {
            line.strip_prefix('#')?
                .split_once('=')?
                .0
                .parse::<u64>()
                .ok()
        })
        .max()
        .unwrap();
    let mut data = Data {
        text: String::new(),
        last,
    };

    let origin = data.point([0.0; 3]);
    let identity = data.add(&format!(
        "CARTESIAN_TRANSFORMATION_OPERATOR_3D('','',$,$,#{origin},1.,$)"
    ));
    let chain = |data: &mut Data, kind: &str, start: &str| -> String {
        let mut top: u64 = start.trim_start_matches('#').parse().unwrap();

        for _ in 0..REPLICA_CHAIN {
            top = data.add(&format!("{kind}('',#{top},#{identity})"));
        }
        format!("#{top}")
    };

    let mut lines = Vec::new();

    for line in text.lines() {
        let rewired = if let Some((head, point)) = line.split_once("=VERTEX_POINT('',") {
            let point = point.trim_end_matches(");");

            format!(
                "{head}=VERTEX_POINT('',{});",
                chain(&mut data, "POINT_REPLICA", point)
            )
        } else if let Some((head, fields)) = line.split_once("=EDGE_CURVE('',") {
            let [start, end, curve, sense] =
                fields.trim_end_matches(");").split(',').collect::<Vec<_>>()[..]
            else {
                panic!("{line}");
            };
            let curve = chain(&mut data, "CURVE_REPLICA", curve);

            format!("{head}=EDGE_CURVE('',{start},{end},{curve},{sense});")
        } else {
            line.to_owned()
        };

        lines.push(rewired);
    }

    let rewired = lines.join("\n") + "\n";
    let end = rewired.rfind("ENDSEC;").unwrap();

    [&rewired[..end], &data.text, &rewired[end..]].concat()
}

fn generated() -> [(&'static str, String); 5] {
    [
        ("styles-fanned.step", styles_fanned()),
        ("assembly-layered.step", assembly_layered()),
        ("assembly-mapped.step", assembly_mapped()),
        ("assembly-deep.step", assembly_deep()),
        ("replicas-deep.step", replicas_deep()),
    ]
}

#[test]
#[ignore = "writes the STEP fuzz seeds when CADITOR_WRITE_FUZZ_SEEDS is set"]
fn write_step_fuzz_seeds() {
    if std::env::var_os(WRITE_SEEDS).is_none() {
        return;
    }

    let dir = seeds_dir();

    fs::create_dir_all(&dir).unwrap();
    for (name, text) in generated() {
        fs::write(dir.join(name), text).unwrap();
    }
}

fn copies_were_cut_short(notes: &[String]) -> bool {
    notes
        .iter()
        .any(|note| note.starts_with(PLACEMENTS_CUT_SHORT))
}

#[test]
fn step_seeds_are_read_within_their_work_bound() {
    let mut seeds: Vec<PathBuf> = fs::read_dir(seeds_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();

    seeds.sort();

    assert!(!seeds.is_empty());
    for path in seeds {
        let text = fs::read_to_string(&path).unwrap();

        let (read, work) = tally::measure(|| read_step(&text));

        let refused = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(REFUSED));
        let read_through = WORK_PER_BYTE * text.len();

        let bound = match &read {
            Ok(model) => {
                assert!(!refused, "{} was read", path.display());
                if copies_were_cut_short(&model.notes) {
                    read_through + MAX_WALKED_PLACEMENTS + MAX_INSTANCES
                } else {
                    read_through
                }
            }
            Err(error) => {
                assert!(refused, "{}: {error}", path.display());
                assert!(!error.to_string().is_empty(), "{}", path.display());
                read_through + MAX_WALKED_PLACEMENTS + MAX_INSTANCES
            }
        };

        assert!(work <= bound, "{}: {work} > {bound}", path.display());
    }
}
