use borsh::{BorshDeserialize, BorshSerialize};
use std::time::Duration;

pub mod borsh_duration {
    use super::*;
    use std::io::{Read, Result, Write};

    pub fn serialize<W: Write>(duration: &Duration, writer: &mut W) -> Result<()> {
        BorshSerialize::serialize(&duration.as_secs(), writer)?;
        BorshSerialize::serialize(&duration.subsec_nanos(), writer)?;
        Ok(())
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Duration> {
        let secs: u64 = BorshDeserialize::deserialize_reader(reader)?;
        let nanos: u32 = BorshDeserialize::deserialize_reader(reader)?;
        Ok(Duration::new(secs, nanos))
    }
}

pub mod borsh_option_duration {
    use super::*;
    use std::io::{self, ErrorKind, Read, Result, Write};

    pub fn serialize<W: Write>(duration: &Option<Duration>, writer: &mut W) -> Result<()> {
        match duration {
            Some(d) => {
                BorshSerialize::serialize(&1u8, writer)?;
                super::borsh_duration::serialize(d, writer)
            }
            None => BorshSerialize::serialize(&0u8, writer),
        }
    }

    pub fn deserialize<R: Read>(reader: &mut R) -> Result<Option<Duration>> {
        let tag: u8 = BorshDeserialize::deserialize_reader(reader)?;
        match tag {
            0 => Ok(None),
            1 => {
                let d = super::borsh_duration::deserialize(reader)?;
                Ok(Some(d))
            }
            _ => Err(io::Error::new(
                ErrorKind::InvalidData,
                "Invalid tag for Option<Duration>",
            )),
        }
    }
}
