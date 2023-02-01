use near_sdk::borsh::{self, BorshDeserialize, BorshSerialize};
use near_sdk::collections::{LookupMap, LookupSet, UnorderedMap, UnorderedSet};
use near_sdk::json_types::{U128, U64};
use near_sdk::serde::{Deserialize, Serialize};
use near_sdk::{
    assert_one_yocto, env, ext_contract, near_bindgen, require, AccountId, Balance,
    BorshStorageKey, CryptoHash, Gas, PanicOnDefault, Promise
};
use std::collections::HashMap;

use crate::external::*;
use crate::internal::*;
use crate::sale::*;
use near_sdk::env::STORAGE_PRICE_PER_BYTE;

mod external;
mod internal;
mod nft_callbacks;
mod sale;
mod sale_views;

const GAS_FOR_RESOLVE_PURCHASE: Gas = Gas(115_000_000_000_000);
const GAS_FOR_NFT_TRANSFER: Gas = Gas(15_000_000_000_000);
const GAS_FOR_FT_TRANSFER: Gas = Gas(5_000_000_000_000);
const STORAGE_PER_SALE: u128 = 1000 * STORAGE_PRICE_PER_BYTE;
static DELIMETER: &str = ".";

pub type SalePriceInTokens = U128;
pub type TokenId = String;
pub type FungibleTokenId = AccountId;
pub type ContractAndTokenId = String;

#[derive(Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Payout {
    pub payout: HashMap<AccountId, U128>,
}

#[near_bindgen]
#[derive(BorshDeserialize, BorshSerialize, PanicOnDefault)]
pub struct Contract {
    pub owner_id: AccountId,

    pub sales: UnorderedMap<ContractAndTokenId, Sale>,

    pub by_owner_id: LookupMap<AccountId, UnorderedSet<ContractAndTokenId>>,

    pub by_nft_contract_id: LookupMap<AccountId, UnorderedSet<TokenId>>,

    pub approved_ft_tokens: LookupSet<FungibleTokenId>,

    pub storage_deposits: LookupMap<AccountId, Balance>,
}

#[derive(BorshStorageKey, BorshSerialize)]
pub enum StorageKey {
    Sales,
    ByOwnerId,
    ByOwnerIdInner { account_id_hash: CryptoHash },
    ByNFTContractId,
    ByNFTContractIdInner { account_id_hash: CryptoHash },
    FTTokenIds,
    StorageDeposits,
}

#[near_bindgen]
impl Contract {
    /*
        Initializes the contract with default data and the owner ID
    */
    #[init]
    pub fn new(owner_id: AccountId) -> Self {
        let mut this = Self {
            owner_id,
            sales: UnorderedMap::new(StorageKey::Sales),
            by_owner_id: LookupMap::new(StorageKey::ByOwnerId),
            by_nft_contract_id: LookupMap::new(StorageKey::ByNFTContractId),
            approved_ft_tokens: LookupSet::new(StorageKey::FTTokenIds),
            storage_deposits: LookupMap::new(StorageKey::StorageDeposits),
        };

        let near_contract_id = AccountId::new_unchecked("near".to_string());
        this.approved_ft_tokens.insert(&near_contract_id);
        this
    }

    pub fn add_ft_token_ids(&mut self, ft_token_ids: Vec<FungibleTokenId>) -> Vec<bool> {
        require!(env::predecessor_account_id() == self.owner_id,"Only Owner can approve ft_token_id");

        let mut added = vec![];

        for ft_token_id in ft_token_ids {
            added.push(self.approved_ft_tokens.insert(&ft_token_id));
        }
        
        added
    }

    //Cover the cost of storing sale objects on the contract
    //Optional account ID is to users can pay for storage for other people.
    #[payable]
    pub fn storage_deposit(&mut self, account_id: Option<AccountId>) {
        let storage_account_id = account_id
            .map(|a| a.into())
            .unwrap_or_else(env::predecessor_account_id);

        let deposit = env::attached_deposit();

        require!(
            deposit >= STORAGE_PER_SALE,
            format!("Requires minimum deposit of {}", STORAGE_PER_SALE)
        );

        let mut balance: u128 = self.storage_deposits.get(&storage_account_id).unwrap_or(0);
        balance = balance
            .checked_add(deposit)
            .unwrap_or_else(|| env::panic_str("Balance Overflow"));
        self.storage_deposits.insert(&storage_account_id, &balance);
    }

    //Withdraw any excess storage fees.
    #[payable]
    pub fn storage_withdraw(&mut self) {
        assert_one_yocto();

        let owner_id = env::predecessor_account_id();
        let mut amount = self.storage_deposits.remove(&owner_id).unwrap_or(0);

        let sales = self.by_owner_id.get(&owner_id);
        let len = sales.map(|s| s.len()).unwrap_or_default();
        let diff = u128::from(len) * STORAGE_PER_SALE;

        amount -= diff;

        if amount > 0 {
            Promise::new(owner_id.clone()).transfer(amount);
        }

        if diff > 0 {
            self.storage_deposits.insert(&owner_id, &diff);
        }
    }

    pub fn storage_minimum_balance(&self) -> U128 {
        U128(STORAGE_PER_SALE)
    }

    pub fn storage_balance_of(&self, account_id: AccountId) -> U128 {
        U128(self.storage_deposits.get(&account_id).unwrap_or(0))
    }
}
