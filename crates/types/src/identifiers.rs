use crate::{
    keys::{AccountId, Instrument, Symbol, Venue},
    primitive::MarketType,
};
use anyhow::{anyhow, Result};
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, str::FromStr};

#[derive(
    Debug,
    Clone,
    Hash,
    Eq,
    PartialEq,
    Deserialize,
    BorshSerialize,
    BorshDeserialize,
    PartialOrd,
    Ord,
)]
pub struct BalanceKey {
    pub account: Account,
    pub symbol: Symbol,
}

impl FromStr for BalanceKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(4, '|');
        let venue = parts.next().ok_or_else(|| anyhow!("Missing venue"))?.into();
        let market_type = parts
            .next()
            .ok_or_else(|| anyhow!("Missing market type"))?
            .parse()?;

        let account_id = parts
            .next()
            .ok_or_else(|| anyhow!("Missing account_id"))?
            .into();
        let symbol = parts
            .next()
            .ok_or_else(|| anyhow!("Missing symbol"))?
            .into();

        Ok(Self {
            account: Account {
                venue,
                market_type,
                account_id,
            },
            symbol,
        })
    }
}

impl fmt::Display for BalanceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}", self.account, self.symbol)
    }
}

impl Serialize for BalanceKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct Account {
    pub venue: Venue,
    pub market_type: MarketType,
    pub account_id: AccountId,
}

impl FromStr for Account {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(3, '|');
        let venue = parts.next().ok_or_else(|| anyhow!("Missing venue"))?.into();
        let market_type = parts
            .next()
            .ok_or_else(|| anyhow!("Missing market_type"))?
            .parse()?;
        let account_id = parts
            .next()
            .ok_or_else(|| anyhow!("Missing account_id"))?
            .into();
        Ok(Self {
            venue,
            market_type,
            account_id,
        })
    }
}

impl fmt::Display for Account {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}|{}", self.venue, self.market_type, self.account_id)
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct EvmAccount {
    pub venue: Venue,
    pub account_id: AccountId,
}

impl FromStr for EvmAccount {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '|');
        let venue = parts.next().ok_or_else(|| anyhow!("Missing venue"))?.into();
        let account_id = parts
            .next()
            .ok_or_else(|| anyhow!("Missing account_id"))?
            .into();
        Ok(Self { venue, account_id })
    }
}

impl fmt::Display for EvmAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}", self.venue, self.account_id)
    }
}

impl Serialize for EvmAccount {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EvmAccount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String as Deserialize>::deserialize(deserializer)?;
        Self::from_str(&s).map_err(DeError::custom)
    }
}

impl Serialize for Account {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Account {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String as Deserialize>::deserialize(deserializer)?;
        Self::from_str(&s).map_err(DeError::custom)
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct BookKey {
    pub account: Account,
    pub instrument: Instrument,
}

impl FromStr for BookKey {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(4, '|');
        let venue = parts.next().ok_or_else(|| anyhow!("Missing venue"))?.into();
        let market_type = parts
            .next()
            .ok_or_else(|| anyhow!("market type"))?
            .parse()?;
        let account_id = parts
            .next()
            .ok_or_else(|| anyhow!("Missing account_id"))?
            .into();
        let instrument = parts
            .next()
            .ok_or_else(|| anyhow!("Missing instrument"))?
            .into();

        Ok(Self {
            account: Account {
                venue,
                market_type,
                account_id,
            },
            instrument,
        })
    }
}

impl fmt::Display for BookKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}", self.account, self.instrument)
    }
}

impl Serialize for BookKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for BookKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String as Deserialize>::deserialize(deserializer)?;
        Self::from_str(&s).map_err(DeError::custom)
    }
}

#[derive(BorshSerialize, BorshDeserialize, Debug, Clone, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct VenueBookKey {
    pub venue: Venue,
    pub instrument: Instrument,
}

impl FromStr for VenueBookKey {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.splitn(2, '|');
        let venue = parts.next().ok_or("Missing venue")?.into();
        let instrument = parts.next().ok_or("Missing instrument")?.into();

        Ok(Self { venue, instrument })
    }
}

impl fmt::Display for VenueBookKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}|{}", self.venue, self.instrument)
    }
}

impl Serialize for VenueBookKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for VenueBookKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = <String as Deserialize>::deserialize(deserializer)?;
        Self::from_str(&s).map_err(DeError::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_account_from_str() {
        let input = "binance|spot|hotwallet";
        let account = Account::from_str(input).unwrap();
        assert_eq!(account.venue.as_str(), "binance");
        assert_eq!(account.account_id.as_str(), "hotwallet");
    }

    #[test]
    fn test_account_from_str_invalid() {
        let input = "binance_only";
        let result = Account::from_str(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_account_display() {
        let account = Account {
            venue: "binance".into(),
            market_type: MarketType::Spot,
            account_id: "hotwallet".into(),
        };
        assert_eq!(account.to_string(), "binance|spot|hotwallet");
    }

    #[test]
    fn test_account_serde_json() {
        let original = Account {
            venue: "binance".into(),
            market_type: MarketType::Spot,
            account_id: "hotwallet".into(),
        };

        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"binance|spot|hotwallet\"");

        let deserialized: Account = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, original);
    }

    #[test]
    fn test_book_key_from_str() {
        let input = "binance|spot|hotwallet|ETH-USDC";
        let book_key = BookKey::from_str(input).unwrap();

        assert_eq!(book_key.account.venue.as_str(), "binance");
        assert_eq!(book_key.account.account_id.as_str(), "hotwallet");
        assert_eq!(book_key.instrument.as_str(), "ETH-USDC");
    }

    #[test]
    fn test_book_key_from_str_invalid() {
        // Missing instrument part
        let input = "binance|hotwallet";
        assert!(BookKey::from_str(input).is_err());

        // Only venue
        let input = "binance";
        assert!(BookKey::from_str(input).is_err());
    }

    #[test]
    fn test_book_key_display() {
        let book_key = BookKey {
            account: Account {
                venue: "binance".into(),
                account_id: "hotwallet".into(),
                market_type: MarketType::Spot,
            },
            instrument: "ETH-USDC".into(),
        };

        assert_eq!(book_key.to_string(), "binance|spot|hotwallet|ETH-USDC");
    }

    #[test]
    fn test_book_key_serde_json() {
        let original = BookKey {
            account: Account {
                venue: "binance".into(),
                account_id: "hotwallet".into(),
                market_type: MarketType::Spot,
            },
            instrument: "ETH-USDC".into(),
        };

        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"binance|spot|hotwallet|ETH-USDC\"");

        let deserialized: BookKey = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, original);
    }

    #[test]
    fn test_account_as_key() {
        let mut map = HashMap::new();
        let account = Account {
            venue: "venue".into(),
            account_id: "acocunt".into(),

            market_type: MarketType::Spot,
        };
        map.insert(account, "test");

        let ser = toml::to_string(&map).unwrap();

        println!("{ser:?}");
    }
}
