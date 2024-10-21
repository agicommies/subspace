use crate::*;
use frame_support::{
    pallet_prelude::ValueQuery,
    traits::{ConstU32, Get, StorageVersion},
};

pub mod v4 {
    use frame_support::{traits::OnRuntimeUpgrade, weights::Weight};

    use super::*;

    pub mod old_storage {
        use super::*;
        use dao::ApplicationStatus;
        use frame_support::{pallet_prelude::TypeInfo, storage_alias, Identity};
        use pallet_subspace::AccountIdOf;
        use parity_scale_codec::{Decode, Encode};
        use sp_runtime::BoundedVec;

        #[derive(Encode, Decode, TypeInfo)]
        pub struct CuratorApplication<T: Config> {
            pub id: u64,
            pub user_id: T::AccountId,
            pub paying_for: T::AccountId,
            pub data: BoundedVec<u8, ConstU32<256>>,
            pub status: ApplicationStatus,
            pub application_cost: u64,
        }

        #[storage_alias]
        pub type CuratorApplications<T: Config> =
            StorageMap<Pallet<T>, Identity, u64, CuratorApplication<T>>;

        #[storage_alias]
        pub type LegitWhitelist<T: Config> =
            StorageMap<Pallet<T>, Identity, AccountIdOf<T>, u8, ValueQuery>;
    }

    pub struct MigrateToV4<T>(sp_std::marker::PhantomData<T>);

    impl<T: Config> OnRuntimeUpgrade for MigrateToV4<T> {
        fn on_runtime_upgrade() -> frame_support::weights::Weight {
            let on_chain_version = StorageVersion::get::<Pallet<T>>();
            if on_chain_version != 3 {
                log::info!("Storage v2 already updated");
                return Weight::zero();
            }

            StorageVersion::new(4).put::<Pallet<T>>();

            let _ = Proposals::<T>::clear(u32::MAX, None);

            log::info!("Migrated to v4");

            T::DbWeight::get().reads_writes(2, 2)
        }
    }
}
