use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomProperties {
    values: BTreeMap<String, String>,
}

impl CustomProperties {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    pub fn set(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.values.insert(name.into(), value.into());
    }

    pub fn inherit_from(&mut self, inherited: &Self) {
        for (name, value) in &inherited.values {
            self.values
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CustomProperties;

    #[test]
    fn custom_properties_inherit_without_overwriting_local_values() {
        let mut inherited = CustomProperties::default();
        inherited.set("--animation-color", "#2563eb");
        inherited.set("--animation-speed", "180ms");

        let mut local = CustomProperties::default();
        local.set("--animation-speed", "240ms");
        local.set("--card-radius", "12px");

        local.inherit_from(&inherited);

        assert_eq!(local.get("--animation-color"), Some("#2563eb"));
        assert_eq!(local.get("--animation-speed"), Some("240ms"));
        assert_eq!(local.get("--card-radius"), Some("12px"));
    }
}
