use crate::{
    net::{ClientPlatform, DisconnectReason, GameProfile},
    server::Server,
};
use pumpkin_util::text::TextComponent;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct AdmissionReservations {
    identities: Mutex<HashMap<Uuid, String>>,
}

/// Holds exclusive access to a player's UUID and name until disconnect cleanup and saves finish.
pub struct AdmissionReservation {
    reservations: Arc<AdmissionReservations>,
    uuid: Uuid,
}

impl AdmissionReservations {
    /// Checks every transport's admission policy and reserves its UUID and name before player-data access.
    pub(crate) fn admit(
        self: &Arc<Self>,
        profile: &GameProfile,
        client: &ClientPlatform,
        server: &Server,
    ) -> Option<AdmissionReservation> {
        // PlayerList.canPlayerLogin/placeNewPlayer share the authenticated profile and address.
        if let Some(reason) = crate::net::can_not_join(profile, &client.address(), server) {
            client.try_kick(DisconnectReason::Kicked, &reason);
            return None;
        }
        let reservation = self.reserve(profile.id, &profile.name);
        if reservation.is_none() {
            client.try_kick(
                DisconnectReason::Kicked,
                &TextComponent::translate("multiplayer.disconnect.duplicate_login", []),
            );
        }
        reservation
    }

    /// Reserves the UUID and case-insensitive name together until the returned guard is dropped.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        uuid: Uuid,
        name: &str,
    ) -> Option<AdmissionReservation> {
        let mut identities = self
            .identities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Fork cross-edition policy uses PlayerList.getPlayerByName's case-insensitive lookup.
        if identities.contains_key(&uuid)
            || identities
                .values()
                .any(|reserved_name| reserved_name.eq_ignore_ascii_case(name))
        {
            return None;
        }
        identities.insert(uuid, name.to_owned());
        Some(AdmissionReservation {
            reservations: self.clone(),
            uuid,
        })
    }
}

impl Drop for AdmissionReservation {
    fn drop(&mut self) {
        self.reservations
            .identities
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.uuid);
    }
}

impl ClientPlatform {
    fn admission_slot(&self) -> &Mutex<Option<AdmissionReservation>> {
        match self {
            Self::Java(client) => &client.admission_reservation,
            Self::Bedrock(client) => &client.admission_reservation,
        }
    }

    /// Retains the admitted UUID and name until disconnect cleanup explicitly releases them.
    pub(crate) fn retain_admission(&self, reservation: AdmissionReservation) {
        *self
            .admission_slot()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reservation);
    }

    /// Releases the UUID and name only after world removal and player-data saves have finished.
    pub(crate) fn release_admission(&self) {
        let reservation = self
            .admission_slot()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(reservation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_identity_admission_is_exclusive_and_released_on_cleanup() {
        let reservations = Arc::new(AdmissionReservations::default());
        let barrier = Arc::new(std::sync::Barrier::new(5));
        let uuid = Uuid::nil();
        let threads: [_; 4] = std::array::from_fn(|_| {
            let reservations = reservations.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let guard = reservations.reserve(uuid, "Steve");
                barrier.wait();
                guard
            })
        });
        barrier.wait();
        let guards: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(guards.iter().filter(|guard| guard.is_some()).count(), 1);
        assert!(reservations.reserve(uuid, "Steve").is_none());
        drop(guards);
        assert!(reservations.reserve(uuid, "Steve").is_some());
    }

    #[test]
    fn concurrent_same_name_admission_is_exclusive_across_uuids() {
        let reservations = Arc::new(AdmissionReservations::default());
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let threads =
            [(Uuid::from_u128(1), "Notch"), (Uuid::from_u128(2), "notch")].map(|(uuid, name)| {
                let reservations = reservations.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let guard = reservations.reserve(uuid, name);
                    barrier.wait();
                    guard
                })
            });
        barrier.wait();
        let guards: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(guards.iter().filter(|guard| guard.is_some()).count(), 1);
        assert!(reservations.reserve(Uuid::from_u128(3), "NOTCH").is_none());
        let other_name = reservations.reserve(Uuid::from_u128(4), "Alex");
        assert!(other_name.is_some());
        drop(guards);
        assert!(reservations.reserve(Uuid::from_u128(3), "NOTCH").is_some());
    }
}
