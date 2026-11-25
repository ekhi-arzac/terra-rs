use crate::wire::{Error, Reader, put_string};
use bytes::{BufMut, BytesMut};

/// The 1.4.6 status sent to the client as soon as it connects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    pub port: i32,
    pub world: String,
    pub host: String,
    pub width: u16,
    pub crimson: bool,
    pub difficulty: i32,
    pub capacity: u8,
    pub players: u8,
    pub note: String,
    pub mapped_address: Option<(String, i32)>,
}
impl ServerInfo {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        Ok(Self {
            port: r.i32()?,
            world: r.string()?.to_owned(),
            host: r.string()?.to_owned(),
            width: r.u16()?,
            crimson: r.byte()? != 0,
            difficulty: r.i32()?,
            capacity: r.byte()?,
            players: r.byte()?,
            note: r.string()?.to_owned(),
            mapped_address: if r.byte()? != 0 {
                Some((r.string()?.to_owned(), r.i32()?))
            } else {
                None
            },
        })
    }
    pub(crate) fn write(&self, out: &mut BytesMut) -> Result<(), Error> {
        out.put_i32_le(self.port);
        put_string(out, &self.world)?;
        put_string(out, &self.host)?;
        out.put_u16_le(self.width);
        out.put_u8(self.crimson.into());
        out.put_i32_le(self.difficulty);
        out.put_u8(self.capacity);
        out.put_u8(self.players);
        put_string(out, &self.note)?;
        out.put_u8(self.mapped_address.is_some().into());
        if let Some((address, port)) = &self.mapped_address {
            put_string(out, address)?;
            out.put_i32_le(*port);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerPlatform {
    pub player: u8,
    pub platform: u8,
    pub id: String,
    /// Platform ID 5 carries a display name on the wire.
    pub name: String,
    pub host: bool,
}
impl PlayerPlatform {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self, Error> {
        let player = r.byte()?;
        let platform = r.byte()?;
        let id = r.string()?.to_owned();
        let name = if platform == 5 {
            r.string()?.to_owned()
        } else {
            String::new()
        };
        Ok(Self {
            player,
            platform,
            id,
            name,
            host: r.byte()? != 0,
        })
    }
    pub(crate) fn write(&self, out: &mut BytesMut) -> Result<(), Error> {
        out.put_u8(self.player);
        out.put_u8(self.platform);
        put_string(out, &self.id)?;
        if self.platform == 5 {
            put_string(out, &self.name)?;
        }
        out.put_u8(self.host.into());
        Ok(())
    }
}
