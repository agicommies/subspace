use super::*;
use frame_support::{storage::with_storage_layer, traits::Get, weights::Weight};
use pallet_subnet_emission_api::SubnetConsensus;
use pallet_subspace::{Keys, Pallet as PalletS, Uids};
use scale_info::prelude::collections::BTreeSet;

impl<T: Config> Pallet<T> {
    pub(crate) fn deregister_not_whitelisted_modules(mut remaining: Weight) -> Weight {
        let netuid = Self::get_consensus_netuid(SubnetConsensus::Linear).unwrap_or(2);

        const MAX_MODULES: usize = 5;

        let db_weight = T::DbWeight::get();

        let mut weight = db_weight.reads(2);

        let find_id_weight = db_weight.reads(1);
        let deregister_weight = Weight::from_parts(300_495_000, 21587)
            .saturating_add(T::DbWeight::get().reads(34_u64))
            .saturating_add(T::DbWeight::get().writes(48_u64));

        if !remaining
            .all_gte(weight.saturating_add(find_id_weight).saturating_add(deregister_weight))
        {
            log::info!("not enough weight remaining: {remaining:?}");
            return Weight::zero();
        }

        let s0_keys: BTreeSet<_> = Keys::<T>::iter_prefix_values(netuid).collect();
        let whitelisted = T::whitelisted_keys();

        let not_whitelisted = s0_keys.difference(&whitelisted);

        remaining = remaining.saturating_sub(weight);

        for not_whitelisted in not_whitelisted.take(MAX_MODULES) {
            log::info!("deregistering module {not_whitelisted:?}");

            // we'll need at least to read outbound lane state, kill a message and update lane
            // state
            if !remaining.all_gte(find_id_weight.saturating_add(deregister_weight)) {
                log::info!("not enough weight remaining: {remaining:?}");
                break;
            }

            let uid = Uids::<T>::get(netuid, not_whitelisted);
            weight = weight.saturating_add(find_id_weight);
            remaining = remaining.saturating_sub(find_id_weight);

            if let Some(uid) = uid {
                let Err(err) =
                    with_storage_layer(|| PalletS::<T>::remove_module(netuid, uid, true))
                else {
                    weight = weight.saturating_add(deregister_weight);
                    remaining = remaining.saturating_sub(deregister_weight);
                    continue;
                };

                log::error!("failed to deregister module {uid} due to: {err:?}");
            }
        }

        weight
    }
}
