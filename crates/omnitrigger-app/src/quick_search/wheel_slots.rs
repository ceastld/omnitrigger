use crate::{
    wheel_scenes::WheelSlotKey,
    wheel_workspace::RING_NAMES,
    wheels::{WheelDefinition, WheelSlot},
};

use super::{title_rank, Destination};

pub(super) struct SearchWheelSlot {
    key: WheelSlotKey,
    label: String,
    detail: String,
    label_key: String,
    search_key: String,
}

impl SearchWheelSlot {
    fn new(
        wheel: &WheelDefinition,
        application: Option<&str>,
        index: usize,
        slot: &WheelSlot,
    ) -> Self {
        let scope = match (application, slot.scene.as_deref()) {
            (Some(process), _) => process.to_owned(),
            (None, Some(scene)) => format!("场景 {scene}"),
            (None, None) => "通用".into(),
        };
        let (ring, sector) = slot.position.map_or((0, index), |position| {
            (usize::from(position.ring), usize::from(position.sector))
        });
        let ring = RING_NAMES.get(ring).copied().unwrap_or("槽位");
        let detail = format!("{} · {scope} · {ring}第{}格", wheel.name, sector + 1);
        Self {
            key: WheelSlotKey::new(wheel, application, index),
            label: slot.label.clone(),
            label_key: slot.label.to_lowercase(),
            search_key: format!("{} {} {detail}", slot.label, wheel.id).to_lowercase(),
            detail,
        }
    }
}

pub(super) fn index_slots(wheels: &[WheelDefinition]) -> Vec<SearchWheelSlot> {
    let mut result = Vec::new();
    for wheel in wheels {
        result.extend(
            wheel
                .slots
                .iter()
                .enumerate()
                .map(|(index, slot)| SearchWheelSlot::new(wheel, None, index, slot)),
        );
        for application in &wheel.app_overrides {
            result.extend(
                application
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| slot.scene.is_none())
                    .map(|(index, slot)| {
                        SearchWheelSlot::new(wheel, Some(&application.process), index, slot)
                    }),
            );
        }
    }
    result
}

pub(super) fn matching_slots(
    slots: &[SearchWheelSlot],
    query: &str,
    limit: usize,
) -> Vec<Destination> {
    let query = query.to_lowercase();
    let mut matched: Vec<_> = slots
        .iter()
        .filter(|slot| {
            query
                .split_whitespace()
                .all(|word| slot.search_key.contains(word))
        })
        .collect();
    matched.sort_by_key(|slot| title_rank(&slot.label_key, &query));
    matched
        .into_iter()
        .take(limit)
        .map(|slot| Destination::WheelSlot {
            key: slot.key.clone(),
            label: slot.label.clone(),
            detail: slot.detail.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quick_search::{RecentDestinations, Results};
    use crate::wheels::{WheelApplicationOverride, Wheels};

    #[test]
    fn finds_the_named_slot_in_the_expected_layout() {
        let mut wheel = Wheels::default().wheels.remove(0);
        wheel.slots[0].label = "通用复制".into();
        let mut application = WheelApplicationOverride {
            process: "Chrome.exe".into(),
            slots: wheel.slots.clone(),
        };
        application.slots[0].label = "网页复制".into();
        wheel.app_overrides.push(application);

        let slots = index_slots(std::slice::from_ref(&wheel));
        for (query, app, expected) in [
            ("编辑轨道 通用复制", None, "通用复制"),
            ("CHROME.EXE 复制", Some("chrome.exe"), "网页复制"),
        ] {
            assert!(matches!(matching_slots(&slots, query, 4).as_slice(), [
                Destination::WheelSlot { key, label, .. }
            ] if *key == WheelSlotKey::new(&wheel, app, 0) && label == expected));
        }
        assert!(matching_slots(&slots, "不存在的槽位", 4).is_empty());
        let results = Results::for_query(
            "槽位：CHROME.EXE 复制",
            &[],
            &[],
            &slots,
            &[],
            &[],
            &RecentDestinations::default(),
        );
        assert!(
            matches!(results, Results::Destinations(items) if matches!(items.as_slice(), [
            Destination::WheelSlot { label, .. }
        ] if label == "网页复制"))
        );
    }
}
