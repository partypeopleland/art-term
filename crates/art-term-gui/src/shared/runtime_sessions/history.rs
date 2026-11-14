use super::*;

impl RuntimeSessions {
    pub async fn history_request(
        &self,
        hosts: &RuntimeHosts,
        id: &str,
        request: HistoryRequest,
    ) -> Reply<HistoryReply> {
        let conn = self.connection(id)?;
        let Some(binding) = conn.history_binding.as_ref() else {
            return Ok(HistoryReply::Unsupported);
        };
        let Some(key) = history::key_from_name(&binding.name) else {
            return Ok(HistoryReply::Unsupported);
        };
        let version =
            history::HistoryVersion::from_name(&binding.name).ok_or(SessionError::Protocol)?;
        if matches!(request, HistoryRequest::Follow { input: Some(_) }) {
            return Err(SessionError::Protocol.into());
        }
        let native = matches!(
            request,
            HistoryRequest::Scroll { .. } | HistoryRequest::Follow { .. }
        );
        let closing_history = matches!(request, HistoryRequest::Close);
        let was_uncertain = conn.native_uncertain.load(Ordering::SeqCst);
        if native {
            // Prewarm must fence input too: another retained helper may already own copy-mode.
            conn.native_uncertain.store(true, Ordering::SeqCst);
            conn.native_enabled.store(true, Ordering::SeqCst);
        }
        if closing_history {
            conn.history_cancel.send_modify(|n| *n = n.wrapping_add(1));
        }
        let history_generation = *conn.history_cancel.borrow();
        let _interaction =
            if native || closing_history || conn.native_enabled.load(Ordering::SeqCst) {
                Some(conn.native_interaction.lock().await)
            } else {
                None
            };
        if *conn.history_cancel.borrow() != history_generation {
            return Err(SessionError::Cancelled.into());
        }
        let mut slot = if closing_history {
            conn.history.lock().await
        } else {
            conn.history.try_lock().map_err(|_| SessionError::Busy)?
        };
        if closing_history {
            if let Some(mut owned) = slot.take() {
                if owned.channel.close().await {
                    conn.native_held.store(false, Ordering::SeqCst);
                    conn.native_uncertain.store(false, Ordering::SeqCst);
                }
            }
            return Ok(HistoryReply::Unsupported);
        }
        if conn.closed.load(Ordering::SeqCst) {
            return Err(SessionError::Cancelled.into());
        }
        if slot.is_none()
            && !matches!(
                request,
                HistoryRequest::Viewport
                    | HistoryRequest::Scroll { .. }
                    | HistoryRequest::Follow { .. }
            )
        {
            return Ok(HistoryReply::Error {
                error: "expired".into(),
            });
        }
        // The frozen profile stays available while recovery temporarily takes
        // the work handle. Such a pause is retryable, never a new binding.
        if conn
            .handle
            .lock()
            .map_err(|_| SessionError::Internal)?
            .is_none()
        {
            return Err(SessionError::Busy.into());
        }
        let store = hosts.store().map_err(SessionError::from)?;
        if !store
            .is_current(&binding.profile)
            .map_err(SessionError::from)?
        {
            return Err(SessionError::StateChanged.into());
        }
        let snapshot = store
            .snapshot(&binding.profile.id)
            .map_err(SessionError::from)?;
        if snapshot.profile != binding.profile {
            return Err(SessionError::StateChanged.into());
        }
        let mut cancel = conn.cancel.subscribe();
        let mut history_cancel = conn.history_cancel.subscribe();
        let generation = conn.next_generation.load(Ordering::SeqCst);
        let operation = async {
            if slot.is_none() {
                let host_key = ownership_key(&binding.profile);
                let semaphore = {
                    let mut slots = self
                        .history_slots
                        .lock()
                        .map_err(|_| SessionError::Internal)?;
                    if slots.len() >= 512 && !slots.contains_key(&host_key) {
                        return Err(SessionError::Capacity);
                    }
                    slots
                        .entry(host_key.clone())
                        .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(2)))
                        .clone()
                };
                let permit = semaphore
                    .try_acquire_owned()
                    .map_err(|_| SessionError::Capacity)?;
                let mut opening = OpeningHistory {
                    active: self.history_active.clone(),
                    registered: Arc::new(Mutex::new(None)),
                    armed: true,
                };
                let ownership = Arc::new(HistoryOwnership {
                    owned: OwnedShells {
                        shells: self.history_owned.clone(),
                        profile_id: host_key.clone(),
                    },
                    active: self.history_active.clone(),
                    registered: opening.registered.clone(),
                });
                let pending = self
                    .history_owned
                    .lock()
                    .map_err(|_| SessionError::Internal)?
                    .get(&host_key)
                    .cloned()
                    .unwrap_or_default();
                let active = self
                    .history_active
                    .lock()
                    .map_err(|_| SessionError::Internal)?
                    .clone();
                let api = hosts.admin().map_err(SessionError::from)?;
                for name in pending
                    .into_iter()
                    .filter(|name| !active.contains(&format!("{host_key}:{name}")))
                    .take(4)
                {
                    if matches!(
                        tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            api.delete(&snapshot, &name)
                        )
                        .await,
                        Ok(Ok(())) | Ok(Err(ApiError::SessionMissing))
                    ) {
                        ownership.unmark(&name);
                    }
                }
                let channel = HistoryChannel::open(
                    &api,
                    snapshot,
                    &key,
                    version,
                    ownership,
                    |binding, token, since, generation| {
                        hosts.attach_token(binding, token, since, generation)
                    },
                )
                .await?;
                let active_key = format!("{host_key}:{}", channel.helper_name());
                self.history_active
                    .lock()
                    .map_err(|_| SessionError::Internal)?
                    .insert(active_key.clone());
                *slot = Some(HistorySlot {
                    channel,
                    _permit: permit,
                    active: self.history_active.clone(),
                    active_key,
                });
                opening.armed = false;
            }
            slot.as_mut()
                .ok_or(SessionError::Internal)?
                .channel
                .request(request)
                .await
        };
        let result = tokio::select! { biased;
            _ = cancel.changed() => Err(SessionError::Cancelled),
            _ = history_cancel.changed() => Err(SessionError::Cancelled),
            result = operation => result,
        };
        let current = !conn.closed.load(Ordering::SeqCst)
            && generation == conn.next_generation.load(Ordering::SeqCst)
            && *conn.history_cancel.borrow() == history_generation
            && store
                .is_current(&binding.profile)
                .map_err(SessionError::from)?;
        if result.is_err() || !current {
            if let Some(mut owned) = slot.take() {
                if owned.channel.close().await {
                    conn.native_held.store(false, Ordering::SeqCst);
                    conn.native_uncertain.store(false, Ordering::SeqCst);
                }
            }
        }
        if !current {
            return Err(SessionError::StateChanged.into());
        }
        if let Ok(HistoryReply::View { held, .. }) = &result {
            conn.native_held.store(*held, Ordering::SeqCst);
            conn.native_uncertain.store(false, Ordering::SeqCst);
        }
        match result {
            Ok(HistoryReply::Unsupported) => {
                // An old helper never entered native mode; retain any earlier ambiguous lease fence.
                if native
                    && !was_uncertain
                    && slot
                        .as_ref()
                        .is_some_and(|owned| !owned.channel.native_supported())
                {
                    conn.native_uncertain.store(false, Ordering::SeqCst);
                }
                Ok(HistoryReply::Error {
                    error: "unsupported".into(),
                })
            }
            Err(SessionError::Missing) => Ok(HistoryReply::Error {
                error: "unavailable".into(),
            }),
            Err(SessionError::UpgradeRequired) => Ok(HistoryReply::Error {
                error: "unsupported".into(),
            }),
            other => other.map_err(Into::into),
        }
    }
}
