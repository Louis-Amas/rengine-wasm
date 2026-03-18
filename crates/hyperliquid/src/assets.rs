use phf::phf_map;

pub(crate) static ASSETS_TO_IDS: phf::Map<&'static str, u32> = phf_map! {
    "btc" => 0,
    "BTC" => 0,
    "eth" => 1,
    "ETH" => 1,
    "@151" => 10_151,
    "tsla" => 120000
};
