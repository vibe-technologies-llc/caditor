use crate::{
    measure::{BETWEEN_TITLE, Measurements, Readout, Relative, SIZE_SEPARATOR, is_pair},
    measure_panel::value_text,
    model::Model,
    selection::Selection,
    units::Units,
};

const DISTANCE: &str = "Distance";
const ANGLE: &str = "Angle";

#[derive(Default)]
pub struct PairReading {
    measurements: Measurements,
}

impl PairReading {
    pub fn refresh(&mut self, model: &Model, selection: &Selection) -> Option<String> {
        if !is_pair(selection) {
            self.measurements.forget();
            return None;
        }
        self.measurements.refresh(model, selection, Relative::WORLD);
        let (readout, _) = self.measurements.shown()?;
        between_text(readout, model.units())
    }
}

pub fn between_text(readout: &Readout, units: Units) -> Option<String> {
    let group = readout
        .groups
        .iter()
        .find(|group| group.title == BETWEEN_TITLE)?;
    let part = |name: &str| {
        let reading = group
            .readings
            .iter()
            .find(|reading| reading.label.starts_with(name))?;
        Some(format!(
            "{name} {}",
            value_text(reading.value, reading.accuracy, units)
        ))
    };
    let parts: Vec<String> = [part(DISTANCE), part(ANGLE)]
        .into_iter()
        .flatten()
        .collect();
    (!parts.is_empty()).then(|| parts.join(SIZE_SEPARATOR))
}
