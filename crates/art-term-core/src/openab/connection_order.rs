use super::*;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionKind {
    Host,
    Agent,
    Screen,
}

fn permutation(
    current: &[String],
    expected: &[String],
    desired: &[String],
) -> Result<Vec<usize>, ApiError> {
    if current != expected {
        return Err(ApiError::StateChanged);
    }
    let unique: std::collections::HashSet<_> = desired.iter().collect();
    if desired.len() != current.len() || unique.len() != desired.len() {
        return Err(ApiError::Protocol);
    }
    desired
        .iter()
        .map(|id| {
            current
                .iter()
                .position(|v| v == id)
                .ok_or(ApiError::Protocol)
        })
        .collect()
}

impl HostStore {
    pub fn reorder_connections(
        &self,
        kind: ConnectionKind,
        expected_ids: &[String],
        ordered_ids: &[String],
    ) -> Result<(), ApiError> {
        if expected_ids.len() > MAX_HOSTS || ordered_ids.len() > MAX_HOSTS {
            return Err(ApiError::Capacity);
        }
        let mut records = self.records.lock().map_err(|_| ApiError::StateChanged)?;
        self.ensure_source()?;
        if kind == ConnectionKind::Screen {
            let desktops = self.desktop_records()?;
            let ids: Vec<_> = desktops.iter().map(|r| r.profile.id.clone()).collect();
            let indexes = permutation(&ids, expected_ids, ordered_ids)?;
            let candidate: Vec<_> = indexes.into_iter().map(|i| desktops[i].clone()).collect();
            if ids != ordered_ids {
                self.save_desktop_records(&records, &candidate)?;
            }
        } else {
            let category = if kind == ConnectionKind::Agent {
                crate::connections::Category::Agent
            } else {
                crate::connections::Category::Host
            };
            let positions: Vec<_> = records
                .iter()
                .enumerate()
                .filter(|(_, r)| {
                    r.profile
                        .metadata
                        .as_ref()
                        .map(|m| m.category)
                        .unwrap_or(crate::connections::Category::Host)
                        == category
                })
                .map(|(i, _)| i)
                .collect();
            let ids: Vec<_> = positions
                .iter()
                .map(|&i| records[i].profile.id.clone())
                .collect();
            let indexes = permutation(&ids, expected_ids, ordered_ids)?;
            let mut candidate = records.clone();
            for (&position, source) in positions.iter().zip(indexes) {
                candidate[position] = records[positions[source]].clone();
            }
            if ids != ordered_ids {
                self.save(&candidate)?;
                *records = candidate;
            }
        }
        Ok(())
    }
}
