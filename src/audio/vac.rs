use std::collections::{BTreeMap, HashMap, HashSet};

use super::devices::{list_input_devices, list_output_devices, AudioEndpoint, ComGuard};
use super::policy;

#[derive(Clone, Debug)]
pub struct VacCable {
    pub number: u32,
    pub render: AudioEndpoint,
    pub capture: AudioEndpoint,
}

#[derive(Default)]
pub struct VacRouter {
    cables: Vec<VacCable>,
    assignment: HashMap<String, u32>,
    preferences: BTreeMap<String, u32>,
    capture_previous: HashMap<u32, [Option<String>; 3]>,
    capture_targets: HashMap<u32, String>,
    reported: HashSet<String>,
}

impl VacRouter {
    pub fn refresh(&mut self) {
        self.cables = list_cables();
    }

    pub fn load_preferences(&mut self, preferences: impl IntoIterator<Item = (String, u32)>) {
        self.preferences = preferences.into_iter().collect();
        self.assignment.clear();
    }

    pub fn preferences(&self) -> BTreeMap<String, u32> {
        self.preferences.clone()
    }

    pub fn sink(&self) -> Option<&AudioEndpoint> {
        self.cables
            .iter()
            .max_by_key(|cable| cable.number)
            .map(|cable| &cable.render)
    }

    pub fn resolve(&mut self, apps: &[(&str, &str)]) -> HashMap<String, String> {
        let mut routes = HashMap::new();
        for &(app, key) in apps {
            let Some(number) = self.ensure(app, key) else {
                continue;
            };
            let Some(cable) = self.cable(number) else {
                continue;
            };
            routes.insert(app.to_string(), cable.render.id.clone());
        }
        routes
    }

    pub fn apply_policies(&mut self, apps: &[(&str, &str)]) {
        let mut desired = HashMap::new();
        for &(app, key) in apps {
            let Some(number) = self.ensure(app, key) else {
                continue;
            };
            let Some(capture_id) = self.cable(number).map(|cable| cable.capture.id.clone()) else {
                continue;
            };
            let Some(pid) = app
                .strip_prefix("app:")
                .and_then(|pid| pid.parse::<u32>().ok())
            else {
                continue;
            };
            let capture = capture_id
                .strip_prefix("input:")
                .unwrap_or(&capture_id)
                .to_string();
            desired.insert(pid, capture);
        }
        self.reconcile_captures(desired);
    }

    pub fn restore_captures(&mut self) {
        self.reconcile_captures(HashMap::new());
    }

    fn reconcile_captures(&mut self, desired: HashMap<u32, String>) {
        let removed: Vec<u32> = self
            .capture_targets
            .keys()
            .copied()
            .filter(|pid| !desired.contains_key(pid))
            .collect();
        let changed: Vec<(u32, String)> = desired
            .into_iter()
            .filter(|(pid, capture)| self.capture_targets.get(pid) != Some(capture))
            .collect();
        if removed.is_empty() && changed.is_empty() {
            return;
        }
        let _com = ComGuard::new();
        for pid in removed {
            self.capture_targets.remove(&pid);
            if let Some(previous) = self.capture_previous.remove(&pid) {
                match policy::restore_default_capture(pid, &previous) {
                    Ok(()) => crate::logging::log("policy", &format!("pid {pid} entrée restaurée")),
                    Err(error) => crate::logging::log(
                        "policy",
                        &format!("pid {pid} restauration entrée échouée: {error}"),
                    ),
                }
            }
        }
        for (pid, capture) in changed {
            let previous = match self.capture_previous.get(&pid) {
                Some(previous) => previous.clone(),
                None => {
                    let mut previous = policy::get_default_capture(pid);
                    for value in &mut previous {
                        if value.as_deref() == Some(capture.as_str()) {
                            *value = None;
                        }
                    }
                    previous
                }
            };
            match policy::set_default_capture(pid, &capture) {
                Ok(()) => {
                    self.capture_previous.insert(pid, previous);
                    self.capture_targets.insert(pid, capture.clone());
                    self.reported.remove(&format!("{pid}|{capture}"));
                    crate::logging::log("policy", &format!("app:{pid} entrée -> {capture}"));
                }
                Err(error) => {
                    if self.reported.insert(format!("{pid}|{capture}")) {
                        crate::logging::log(
                            "policy",
                            &format!("app:{pid} entrée non assignée: {error}"),
                        );
                    }
                }
            }
        }
    }

    fn ensure(&mut self, app: &str, key: &str) -> Option<u32> {
        let sink = self.cables.iter().map(|cable| cable.number).max();
        if let Some(&number) = self.assignment.get(app) {
            if Some(number) != sink && self.cables.iter().any(|cable| cable.number == number) {
                return Some(number);
            }
            self.assignment.remove(app);
        }
        if let Some(&number) = self.preferences.get(key) {
            let free = Some(number) != sink
                && self.cables.iter().any(|cable| cable.number == number)
                && !self.used_by_other(app, number);
            if free {
                self.assignment.insert(app.to_string(), number);
                return Some(number);
            }
        }
        let used: HashSet<u32> = self
            .assignment
            .iter()
            .filter(|(other, _)| other.as_str() != app)
            .map(|(_, &number)| number)
            .collect();
        let number = self
            .cables
            .iter()
            .map(|cable| cable.number)
            .filter(|number| Some(*number) != sink)
            .find(|number| !used.contains(number));
        let Some(number) = number else {
            if self.reported.insert(format!("{app}|aucun")) {
                crate::logging::log("vac", &format!("{app} sans câble libre"));
            }
            return None;
        };
        self.assignment.insert(app.to_string(), number);
        self.preferences.insert(key.to_string(), number);
        crate::logging::log("vac", &format!("{app} -> câble {number}"));
        Some(number)
    }

    fn used_by_other(&self, app: &str, number: u32) -> bool {
        self.assignment
            .iter()
            .any(|(other, &assigned)| other.as_str() != app && assigned == number)
    }

    fn cable(&self, number: u32) -> Option<&VacCable> {
        self.cables.iter().find(|cable| cable.number == number)
    }
}

pub fn list_cables() -> Vec<VacCable> {
    let renders = list_output_devices();
    let captures = list_input_devices();
    let mut cables = Vec::new();
    for render in renders {
        if !is_vac(&render.name) {
            continue;
        }
        let Some(number) = cable_number(&render.name) else {
            continue;
        };
        let capture = captures
            .iter()
            .filter(|capture| is_vac(&capture.name) && cable_number(&capture.name) == Some(number))
            .min_by_key(|capture| (capture.name != render.name) as u8)
            .cloned();
        if let Some(capture) = capture {
            cables.push(VacCable {
                number,
                render,
                capture,
            });
        }
    }
    cables.sort_by_key(|cable| cable.number);
    cables
}

pub(crate) fn is_vac(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("virtual audio cable") || name.contains("(vac ")
}

fn cable_number(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    if let Some(index) = lower.find("(vac ") {
        let digits: String = name[index + 5..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect();
        return digits.parse().ok();
    }
    let prefix = name.split(" (").next().unwrap_or(name).trim();
    prefix.rsplit(' ').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::devices::EndpointKind;

    fn endpoint(id: &str, name: &str) -> AudioEndpoint {
        AudioEndpoint {
            id: id.to_string(),
            name: name.to_string(),
            kind: EndpointKind::Hardware,
        }
    }

    fn cable(number: u32) -> VacCable {
        VacCable {
            number,
            render: endpoint(&format!("output:r{number}"), &format!("Line {number}")),
            capture: endpoint(&format!("input:c{number}"), &format!("Line {number}")),
        }
    }

    #[test]
    fn parses_vac_cable_numbers() {
        assert_eq!(cable_number("Line 1 (Virtual Audio Cable)"), Some(1));
        assert_eq!(cable_number("Mic 2 (Virtual Audio Cable)"), Some(2));
        assert_eq!(cable_number("audiok Discord (VAC 3)"), Some(3));
        assert_eq!(cable_number("Speakers (Realtek)"), None);
        assert!(is_vac("Line 1 (Virtual Audio Cable)"));
        assert!(is_vac("audiok Discord (VAC 3)"));
        assert!(!is_vac("Speakers (Realtek)"));
    }

    #[test]
    fn assigns_feedback_cables_while_reserving_the_sink() {
        let mut router = VacRouter {
            cables: vec![cable(1), cable(2), cable(3)],
            ..Default::default()
        };

        assert_eq!(
            router.sink().map(|endpoint| endpoint.id.as_str()),
            Some("output:r3")
        );

        let routes = router.resolve(&[("app:10", "app:alpha")]);
        assert_eq!(routes.get("app:10").map(String::as_str), Some("output:r1"));

        let routes = router.resolve(&[("app:11", "app:beta")]);
        assert_eq!(routes.get("app:11").map(String::as_str), Some("output:r2"));

        let routes = router.resolve(&[("app:12", "app:gamma")]);
        assert!(routes.is_empty());

        let routes = router.resolve(&[("app:10", "app:alpha")]);
        assert_eq!(routes.get("app:10").map(String::as_str), Some("output:r1"));
    }

    #[test]
    fn leaves_apps_unrouted_without_cables() {
        let mut router = VacRouter::default();
        assert!(router.resolve(&[("app:10", "app:alpha")]).is_empty());
        assert!(router.sink().is_none());
    }

    #[test]
    fn reuses_persisted_cable_for_stable_key() {
        let mut router = VacRouter {
            cables: vec![cable(1), cable(2), cable(3)],
            ..Default::default()
        };
        router.load_preferences([("app:discord.exe".to_string(), 2)]);

        let routes = router.resolve(&[("app:10", "app:discord.exe")]);
        assert_eq!(routes.get("app:10").map(String::as_str), Some("output:r2"));
    }

    #[test]
    fn records_assignment_for_stable_key() {
        let mut router = VacRouter {
            cables: vec![cable(1), cable(2), cable(3)],
            ..Default::default()
        };

        router.resolve(&[("app:10", "app:discord.exe")]);
        assert_eq!(
            router.preferences().get("app:discord.exe").copied(),
            Some(1)
        );
    }

    #[test]
    fn ignores_persisted_sink_and_assigns_free_cable() {
        let mut router = VacRouter {
            cables: vec![cable(1), cable(2), cable(3)],
            ..Default::default()
        };
        router.load_preferences([("app:discord.exe".to_string(), 3)]);

        let routes = router.resolve(&[("app:10", "app:discord.exe")]);
        assert_eq!(routes.get("app:10").map(String::as_str), Some("output:r1"));
        assert_eq!(
            router.preferences().get("app:discord.exe").copied(),
            Some(1)
        );
    }
}
