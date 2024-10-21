// use core::marker::PhantomData;

use frame_support::traits::StorageVersion;
use pallet_subnet_emission_api::SubnetConsensus;
use pallet_subspace::{MaxAllowedUids, MaxAllowedWeights, MinAllowedWeights};

pub mod v1 {
    use super::*;
    use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};

    pub struct MigrateToV1<T>(sp_std::marker::PhantomData<T>);

    impl<T: Config> OnRuntimeUpgrade for MigrateToV1<T> {
        fn on_runtime_upgrade() -> Weight {
            let on_chain_version = StorageVersion::get::<Pallet<T>>();
            if on_chain_version != 0 {
                log::info!("Storage v1  already updated");
                return Weight::zero();
            }

            let general_subnet_netuid = 2;
            let onchain_netuid = T::get_consensus_netuid(SubnetConsensus::Linear).unwrap_or(2);

            // return early if there is not a match
            if general_subnet_netuid != onchain_netuid {
                log::info!("General subnet netuid does not match onchain netuid");
                return Weight::zero();
            }

            // Clear all of the current weights on subnet 2
            let _ = Weights::<T>::clear_prefix(general_subnet_netuid, u32::MAX, None);
            log::info!("Cleared all weights for subnet 2");

            // Make sure we allow just one weight for the general subnet
            MinAllowedWeights::<T>::set(general_subnet_netuid, 1);
            log::info!("Set min allowed weights for subnet 2");

            // Make sure max allowed weights are same as max allowed uids
            let max_allowed_uids = MaxAllowedUids::<T>::get(general_subnet_netuid);
            MaxAllowedWeights::<T>::set(general_subnet_netuid, max_allowed_uids);
            log::info!("Set max allowed weights for subnet 2");

            log::info!("Migrating storage to v14");
            StorageVersion::new(1).put::<Pallet<T>>();
            Weight::zero()
        }
    }
}
