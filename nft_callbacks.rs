use crate::*;

//keep track of the sale conditions
#[derive(Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
#[serde(crate = "near_sdk::serde")]
pub struct SaleConditionArgs {
    pub price: SalePriceInTokens,
    pub ft_contract_id: FungibleTokenId,
}

#[derive(Serialize, Deserialize, BorshDeserialize, BorshSerialize)]
#[serde(crate = "near_sdk::serde")]
pub struct PurchaseArgs {
    pub nft_contract_id: AccountId,
    pub token_id: TokenId,
}

/*
    Used as the callback from the NFT contract. When nft_approve is
    called, it will fire a cross contract call to this marketplace and this is the function
    that is invoked.
*/
trait NonFungibleTokenApprovalsReceiver {
    fn nft_on_approve(
        &mut self,
        token_id: TokenId,
        owner_id: AccountId,
        approval_id: u64,
        msg: String,
    );
}

#[near_bindgen]
impl NonFungibleTokenApprovalsReceiver for Contract {
    fn nft_on_approve(
        &mut self,
        token_id: TokenId,
        owner_id: AccountId,
        approval_id: u64,
        msg: String,
    ) {
        let nft_contract_id = env::predecessor_account_id();
        let signer_id = env::signer_account_id();
        require!(
            nft_contract_id != signer_id,
            "nft_on_approve should only be called via cross-contract call"
        );
        require!(owner_id == signer_id, "owner_id should be signer_id");

        let storage_amount = self.storage_minimum_balance().0;
        let owner_paid_storage = self.storage_deposits.get(&signer_id).unwrap_or(0);
        let signer_storage_required =
            (self.get_supply_by_owner_id(signer_id).0 + 1) as u128 * storage_amount;

        require!(
            owner_paid_storage >= signer_storage_required,
            format!(
                "Insufficient storage paid: {}, for {} sales at {} rate of per sale",
                owner_paid_storage,
                signer_storage_required / STORAGE_PER_SALE,
                STORAGE_PER_SALE
            )
        );

        let SaleConditionArgs {
            price,
            ft_contract_id,
        } = near_sdk::serde_json::from_str(&msg).expect("Not valid SaleArgs");

        let sale_conditions = SaleConditionArgs {
            price,
            ft_contract_id,
        };

        let contract_and_token_id = format!("{}{}{}", nft_contract_id, DELIMETER, token_id);

        self.sales.insert(
            &contract_and_token_id,
            &Sale {
                owner_id: owner_id.clone(),
                approval_id,
                nft_contract_id: nft_contract_id.to_string(),
                token_id: token_id.clone(),
                sale_conditions,
            },
        );

        let mut by_owner_id = self.by_owner_id.get(&owner_id).unwrap_or_else(|| {
            UnorderedSet::new(
                StorageKey::ByOwnerIdInner {
                    account_id_hash: hash_account_id(&owner_id),
                }
                .try_to_vec()
                .unwrap(),
            )
        });

        by_owner_id.insert(&contract_and_token_id);
        self.by_owner_id.insert(&owner_id, &by_owner_id);

        let mut by_nft_contract_id = self
            .by_nft_contract_id
            .get(&nft_contract_id)
            .unwrap_or_else(|| {
                UnorderedSet::new(
                    StorageKey::ByNFTContractIdInner {
                        account_id_hash: hash_account_id(&nft_contract_id),
                    }
                    .try_to_vec()
                    .unwrap(),
                )
            });

        by_nft_contract_id.insert(&token_id);

        self.by_nft_contract_id
            .insert(&nft_contract_id, &by_nft_contract_id);
    }
}
