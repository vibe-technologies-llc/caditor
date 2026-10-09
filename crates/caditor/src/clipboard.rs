use std::{
    process,
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::model::Model;

static PROCESS: OnceLock<String> = OnceLock::new();

pub fn source(model: &Model) -> String {
    let process = PROCESS.get_or_init(|| {
        let started = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        format!("{:x}.{started:x}", process::id())
    });
    format!("{process}.{}", model.session())
}
