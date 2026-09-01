//! Privacy settings of [`Telegram`]: `account.getPrivacy` / `account.setPrivacy`.
//!
//! Exposes a coarse "Who can …" panel: last-seen & online status, being added
//! to groups, and phone calls — each as one of three presets (Everyone /
//! Contacts / Nobody) mapped onto Telegram's `InputPrivacyRule`s.

use anyhow::{Context, Result};
use grammers_client::tl::enums::{self, InputPrivacyKey, InputPrivacyRule, PrivacyRule};
use grammers_client::tl::functions::account;

use super::client::Telegram;

/// A privacy category exposed in the Settings → Privacy panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivacyKey {
    /// Who can see your last seen & online status.
    LastSeen,
    /// Who can add you to groups and channels.
    AddToGroups,
    /// Who can call you.
    Calls,
}

/// A coarse access policy, mapped to Telegram's per-key presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivacyPreset {
    Everyone,
    Contacts,
    Nobody,
}

impl PrivacyKey {
    fn tl(self) -> InputPrivacyKey {
        match self {
            PrivacyKey::LastSeen => InputPrivacyKey::StatusTimestamp,
            PrivacyKey::AddToGroups => InputPrivacyKey::ChatInvite,
            PrivacyKey::Calls => InputPrivacyKey::PhoneCall,
        }
    }
}

impl PrivacyPreset {
    fn rule(self) -> InputPrivacyRule {
        match self {
            PrivacyPreset::Everyone => InputPrivacyRule::InputPrivacyValueAllowAll,
            PrivacyPreset::Contacts => InputPrivacyRule::InputPrivacyValueAllowContacts,
            PrivacyPreset::Nobody => InputPrivacyRule::InputPrivacyValueDisallowAll,
        }
    }
    fn from_rules(rules: &[PrivacyRule]) -> Self {
        if rules
            .iter()
            .any(|r| matches!(r, PrivacyRule::PrivacyValueDisallowAll))
        {
            Self::Nobody
        } else if rules
            .iter()
            .any(|r| matches!(r, PrivacyRule::PrivacyValueAllowContacts))
        {
            Self::Contacts
        } else {
            Self::Everyone
        }
    }
}

impl Telegram {
    /// Reads the current effective policy for a setting key.
    pub async fn get_privacy(&self, key: PrivacyKey) -> Result<PrivacyPreset> {
        let res: enums::account::PrivacyRules = self
            .client()
            .invoke(&account::GetPrivacy { key: key.tl() })
            .await
            .context("reading privacy setting")?;
        let rules = match res {
            enums::account::PrivacyRules::Rules(r) => r.rules,
            _ => Vec::new(),
        };
        Ok(PrivacyPreset::from_rules(&rules))
    }

    /// Applies a coarse policy to a setting key.
    pub async fn set_privacy(
        &self,
        key: PrivacyKey,
        preset: PrivacyPreset,
    ) -> Result<()> {
        self.client()
            .invoke(&account::SetPrivacy {
                key: key.tl(),
                rules: vec![preset.rule()],
            })
            .await
            .context("applying privacy setting")?;
        Ok(())
    }
}