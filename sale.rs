use crate::{
    nft_callbacks::{PurchaseArgs, SaleConditionArgs},
    *,
};
use near_sdk::{promise_result_as_success, PromiseOrValue};

/// information about each sale on the market
#[derive(BorshDeserialize, BorshSerialize, Serialize, Deserialize)]
#[serde(crate = "near_sdk::serde")]
pub struct Sale {
    //sale owner
    pub owner_id: AccountId,
    //market contract's approval ID to transfer the token on behalf of the owner
    pub approval_id: u64,
    //nft contract where the token was minted
    pub nft_contract_id: String,
    //actual token ID for sale
    pub token_id: String,
    //sale conditions for token listed
    pub sale_conditions: SaleConditionArgs,
}

#[near_bindgen]
impl Contract {
    //removes a sale from the market.
    #[payable]
    pub fn remove_sale(&mut self, nft_contract_id: AccountId, token_id: String) {
        assert_one_yocto();
        let sale = self.delete_sale(nft_contract_id.into(), token_id);
        let owner_id = env::predecessor_account_id();
        require!(owner_id == sale.owner_id, "Must be sale owner");
    }

    //updates the price for a sale on the market
    #[payable]
    pub fn update_price(
        &mut self,
        nft_contract_id: AccountId,
        token_id: String,
        price: U128,
        ft_contract_id: Option<FungibleTokenId>,
    ) {
        assert_one_yocto();

        let contract_id: AccountId = nft_contract_id.into();
        let contract_and_token_id = format!("{}{}{}", contract_id, DELIMETER, token_id);

        let mut sale = self.sales.get(&contract_and_token_id).expect("No sale");

        require!(
            env::predecessor_account_id() == sale.owner_id,
            "Must be sale owner"
        );

        sale.sale_conditions.price = price;

        if let Some(ft_contract_id) = ft_contract_id {
            require!(
                self.approved_ft_tokens.contains(&ft_contract_id),
                "Only Approved Fungible Tokens can be used for listing"
            );
            sale.sale_conditions.ft_contract_id = ft_contract_id
        }

        self.sales.insert(&contract_and_token_id, &sale);
    }

    //place an offer on a specific sale. The sale will go through as long as your deposit
    // is greater than or equal to the list price
    #[payable]
    pub fn offer(&mut self, nft_contract_id: AccountId, token_id: String) {
        let deposit = env::attached_deposit();
        require!(deposit > 0, "Attached deposit must be greater than 0");

        let contract_id: AccountId = nft_contract_id.into();
        let contract_and_token_id = format!("{}{}{}", contract_id, DELIMETER, token_id);

        let sale = self.sales.get(&contract_and_token_id).expect("No sale");

        let near_contract_id = AccountId::new_unchecked("near".to_string());

        require!(
            sale.sale_conditions.ft_contract_id == near_contract_id,
            "Not available to buy"
        );

        let buyer_id = env::predecessor_account_id();
        require!(sale.owner_id != buyer_id, "Cannot bid on your own sale.");

        let price = sale.sale_conditions.price;

        require!(
            deposit >= price.0,
            format!(
                "Attached deposit must be greater than or equal to the current price: {:?}",
                price
            )
        );

        //process the purchase (which will remove the sale, transfer and get the
        // payout from the nft contract, and then distribute royalties)
        self.process_purchase(contract_id, token_id, U128(deposit),
                              buyer_id, None);
    }

    pub fn ft_on_transfer(
        &mut self,
        sender_id: AccountId,
        amount: U128,
        msg: Option<String>,
    ) -> PromiseOrValue<U128> {
        let msg = if let Some(msg) = msg {
            msg
        } else {
            return PromiseOrValue::Value(amount);
        };

        let PurchaseArgs {
            nft_contract_id,
            token_id,
        } = near_sdk::serde_json::from_str(&msg).expect("Invalid PurchaseArgs");

        let ft_contract_id = env::predecessor_account_id();

        let contract_id: AccountId = nft_contract_id.into();
        let contract_and_token_id = format!("{}{}{}", contract_id, DELIMETER, token_id);

        let sale = self.sales.get(&contract_and_token_id).expect("No sale");

        require!(
            sale.sale_conditions.ft_contract_id == ft_contract_id,
            format!("Cannot Purchase with {} tokens", ft_contract_id)
        );

        let price = sale.sale_conditions.price;

        require!(sale.owner_id != sender_id, "Cannot bid on your own sale.");
        require!(
            price.0 >= amount.0,
            "Attached tokens are less than the listed price"
        );

        PromiseOrValue::Promise(self.process_purchase(
            contract_id,
            token_id,
            amount,
            sender_id,
            Some(ft_contract_id),
        ))
    }

    //remove the sale, transfer and get the payout from the nft contract, and distribute royalties
    #[private]
    pub fn process_purchase(
        &mut self,
        nft_contract_id: AccountId,
        token_id: String,
        price: U128,
        buyer_id: AccountId,
        ft_contract_id: Option<FungibleTokenId>,
    ) -> Promise {
        let sale = self.delete_sale(nft_contract_id.clone(),
                                    token_id.clone());

        ext_contract::ext(nft_contract_id)
            .with_attached_deposit(1)
            .with_static_gas(GAS_FOR_NFT_TRANSFER)
            .nft_transfer_payout(
                buyer_id.clone(),
                token_id,
                sale.approval_id,
                "payout from market".to_string(), //memo (to include some context)
                price,
                7, //the maximum amount of accounts the market can payout at once, limited by GAS
            )
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_FOR_RESOLVE_PURCHASE)
                    .resolve_purchase(buyer_id, price, ft_contract_id),
            )
    }

    /*
        Authenticate the payout object. If everything is fine, it will pay the accounts.
        If there's a problem, refund the buyer for the price.
    */
    #[private]
    pub fn resolve_purchase(
        &mut self,
        buyer_id: AccountId,
        price: U128,
        ft_contract_id: Option<FungibleTokenId>,
    ) -> U128 {
        let payout_option = promise_result_as_success().and_then(|value| {
            near_sdk::serde_json::from_slice::<Payout>(&value)
                .ok()
                .and_then(|payout_object| {
                    if payout_object.payout.len() > 7 || payout_object.payout.is_empty() {
                        env::log_str("Cannot have more than 7 royalties");
                        None
                    } else {
                        let mut remainder = price.0;

                        for &value in payout_object.payout.values() {
                            remainder = remainder.checked_sub(value.0)?;
                        }
                        if remainder == 0 || remainder == 1 {
                            Some(payout_object.payout)
                        } else {
                            None
                        }
                    }
                })
        });

        let payout = if let Some(payout_option) = payout_option {
            payout_option
        } else {
            match ft_contract_id {
                Some(ft_contract_id) => {
                    let memo = Some("Marketplace Refund".to_string());

                    ext_contract::ext(ft_contract_id)
                        .with_attached_deposit(1)
                        .with_static_gas(GAS_FOR_FT_TRANSFER)
                        .ft_transfer(buyer_id, price, memo);
                }
                None => {
                    Promise::new(buyer_id).transfer(u128::from(price));
                }
            }
            return price;
        };

        match ft_contract_id {
            Some(ft_contract_id) => {
                let memo = Some("Marketplace Royalties".to_string());
                for (receiver_id, amount) in payout {
                    ext_contract::ext(ft_contract_id.clone())
                        .with_attached_deposit(1)
                        .with_static_gas(GAS_FOR_FT_TRANSFER)
                        .ft_transfer(receiver_id, amount, memo.clone());
                }
            }
            None => {
                for (receiver_id, amount) in payout {
                    Promise::new(receiver_id).transfer(amount.0);
                }
            }
        }
        price
    }
}

/*
    used to resolve the promise for nft_transfer_payout. Authenticate the payout object.
    If everything is fine, pay the accounts.
    If there's a problem, refund the buyer for the price.
*/
#[ext_contract(ext_self)]
trait ExtSelf {
    fn resolve_purchase(
        &mut self,
        buyer_id: AccountId,
        price: U128,
        ft_contract_id: Option<FungibleTokenId>,
    ) -> Promise;
}
