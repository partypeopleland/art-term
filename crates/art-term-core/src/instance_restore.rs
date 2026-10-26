use crate::instance_mcp::DesktopProfile;
use anyhow::Context;
use serde_json::Value;

pub struct RestoreSelection {
    pub profile: DesktopProfile,
    pub broker_targets: Vec<Value>,
    broker_urls: Value,
}

impl RestoreSelection {
    pub fn confirm(
        self,
        current: &Value,
        latest: &DesktopProfile,
        expected_epoch: u64,
        current_epoch: u64,
        closed: bool,
    ) -> anyhow::Result<DesktopProfile> {
        anyhow::ensure!(!closed && expected_epoch == current_epoch, "桌面恢復已取消");
        anyhow::ensure!(
            current["targets"].as_array() == Some(&self.broker_targets)
                && current["urls"] == self.broker_urls,
            "Shell 工具指派已變更"
        );
        anyhow::ensure!(*latest == self.profile, "桌面設定已變更");
        Ok(self.profile)
    }
}

pub fn resolve(
    status: &Value,
    desktops: &[DesktopProfile],
) -> anyhow::Result<Option<RestoreSelection>> {
    let targets = status["targets"]
        .as_array()
        .context("Shell 工具指派格式不符")?;
    anyhow::ensure!(
        status["ok"] == true && status["adapters"].as_u64() == Some(targets.len() as u64),
        "Shell 工具指派格式不符"
    );
    if targets.is_empty() || targets.iter().all(Value::is_null) {
        return Ok(None);
    }
    let target = targets[0].as_object().context("Shell 工具指派狀態不一致")?;
    let id = target
        .get("id")
        .and_then(Value::as_str)
        .context("Shell 工具指派格式不符")?;
    let revision = target
        .get("revision")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .context("Shell 工具指派格式不符")?;
    anyhow::ensure!(
        targets.iter().all(|item| item["id"] == id
            && item["revision"] == revision
            && item["generation"].as_u64().is_some_and(|n| n > 0)),
        "Shell 工具指派狀態不一致"
    );
    let urls = status
        .get("urls")
        .map(|value| value.as_array().context("Shell 工具來源格式不符"))
        .transpose()?;
    anyhow::ensure!(
        urls.is_none_or(|urls| urls.len() == targets.len()),
        "Shell 工具來源格式不符"
    );
    let source_url = urls.and_then(|urls| urls.first()).and_then(Value::as_str);
    anyhow::ensure!(
        urls.is_none_or(|urls| urls.iter().all(|url| match source_url {
            Some(expected) => url.as_str() == Some(expected),
            None => url.is_null(),
        })),
        "Shell 工具來源不一致"
    );
    let profile = if let Some(local) = desktops.iter().find(|desktop| desktop.id == id) {
        anyhow::ensure!(
            local.revision == revision,
            "Shell 指派版本與本機桌面不符，請重新選擇 Screen"
        );
        anyhow::ensure!(
            source_url.is_none_or(|url| url == local.mcp_url),
            "Shell 指派網址與本機桌面不符"
        );
        local
    } else {
        let url = source_url.context("Shell 指派無法核對，請重新選擇 Screen")?;
        let mut matches = desktops.iter().filter(|desktop| desktop.mcp_url == url);
        let found = matches
            .next()
            .context("Shell 指派的桌面在本機不存在或網址不唯一")?;
        anyhow::ensure!(
            matches.next().is_none(),
            "Shell 指派的桌面在本機不存在或網址不唯一"
        );
        found
    };
    Ok(Some(RestoreSelection {
        profile: profile.clone(),
        broker_targets: targets.clone(),
        broker_urls: status["urls"].clone(),
    }))
}
