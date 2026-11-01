use crate::{
    Result, ServiceError, ServiceErrorCode,
    fonts::{Face, FontCatalog, FontHook},
};
use std::collections::BTreeMap;

#[derive(Default)]
struct Broker {
    families: BTreeMap<String, Vec<Face>>,
    requested: Vec<String>,
    fail: bool,
}
impl FontHook for Broker {
    fn version(&mut self) -> Result<Option<u64>> {
        Ok(Some(1))
    }
    fn families(&mut self) -> Result<Option<Vec<String>>> {
        Ok(Some(self.families.keys().cloned().collect()))
    }
    fn faces(&mut self, family: &str) -> Result<Vec<Face>> {
        self.requested.push(family.into());
        if self.fail {
            return Err(ServiceError::message(
                ServiceErrorCode::Unavailable,
                "CoreText broker unavailable",
            ));
        }
        Ok(self.families.get(family).cloned().unwrap_or_default())
    }
}
fn face(name: &str, weight: f32, italic: bool) -> Face {
    Face {
        name: name.into(),
        weight,
        italic,
    }
}

#[test]
fn family_submenus_and_inspector_receive_the_same_broker_face_facts() {
    let regular = face("Synthetic Regular", 400.0, false);
    let italic = face("Synthetic Italic", 432.5, true);
    let mut catalog = FontCatalog::new(Broker {
        families: BTreeMap::from([
            ("Synthetic".into(), vec![regular.clone(), italic.clone()]),
            ("Other".into(), vec![face("Other Bold", 700.0, false)]),
        ]),
        ..Default::default()
    });
    assert_eq!(catalog.families().unwrap().unwrap(), ["Other", "Synthetic"]);
    assert_eq!(
        catalog.faces("Synthetic").unwrap(),
        [regular, italic.clone()]
    );
    assert_eq!(
        catalog.faces("Other").unwrap(),
        [face("Other Bold", 700.0, false)]
    );
    assert_eq!(catalog.hook.requested, ["Synthetic", "Other"]);
    let roundtrip: Face = serde_json::from_value(serde_json::to_value(&italic).unwrap()).unwrap();
    assert_eq!(roundtrip, italic);
}

#[test]
fn face_snapshots_are_owned_and_activation_changes_are_visible_immediately() {
    let mut catalog = FontCatalog::new(Broker {
        families: BTreeMap::from([(
            "Synthetic".into(),
            vec![face("Synthetic Regular", 400.0, false)],
        )]),
        ..Default::default()
    });
    let mut faces = catalog.faces("Synthetic").unwrap();
    faces[0].name.clear();
    assert_eq!(
        catalog.faces("Synthetic").unwrap()[0].name,
        "Synthetic Regular"
    );
    catalog.hook.families.insert(
        "Synthetic".into(),
        vec![face("Synthetic Bold", 700.0, false)],
    );
    assert_eq!(
        catalog.faces("Synthetic").unwrap(),
        [face("Synthetic Bold", 700.0, false)]
    );
}

#[test]
fn empty_and_failed_face_enumerations_recover_without_invented_faces() {
    let mut catalog = FontCatalog::new(Broker::default());
    assert!(catalog.faces("Synthetic").unwrap().is_empty());
    catalog.hook.fail = true;
    assert_eq!(
        catalog.faces("Synthetic").unwrap_err().code,
        ServiceErrorCode::Unavailable
    );
    catalog.hook.fail = false;
    catalog.hook.families.insert(
        "Synthetic".into(),
        vec![face("Synthetic Regular", 400.0, false)],
    );
    assert_eq!(catalog.faces("Synthetic").unwrap().len(), 1);
}
