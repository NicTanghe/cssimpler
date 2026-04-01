#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransitionPropertyName {
    All,
    Property(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransitionTimingFunction {
    Linear,
    #[default]
    Ease,
    EaseIn,
    EaseOut,
    EaseInOut,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TransitionEntry {
    pub property: TransitionPropertyName,
    pub duration_seconds: f32,
    pub delay_seconds: f32,
    pub timing_function: TransitionTimingFunction,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransitionStyle {
    pub properties: Vec<TransitionPropertyName>,
    pub durations_seconds: Vec<f32>,
    pub delays_seconds: Vec<f32>,
    pub timing_functions: Vec<TransitionTimingFunction>,
}

impl TransitionStyle {
    pub fn entries(&self) -> Vec<TransitionEntry> {
        if self.properties.is_empty() {
            return Vec::new();
        }

        let durations = if self.durations_seconds.is_empty() {
            vec![0.0]
        } else {
            self.durations_seconds.clone()
        };
        let delays = if self.delays_seconds.is_empty() {
            vec![0.0]
        } else {
            self.delays_seconds.clone()
        };
        let timings = if self.timing_functions.is_empty() {
            vec![TransitionTimingFunction::Ease]
        } else {
            self.timing_functions.clone()
        };

        self.properties
            .iter()
            .enumerate()
            .map(|(index, property)| TransitionEntry {
                property: property.clone(),
                duration_seconds: durations[index % durations.len()],
                delay_seconds: delays[index % delays.len()],
                timing_function: timings[index % timings.len()],
            })
            .collect()
    }
}
