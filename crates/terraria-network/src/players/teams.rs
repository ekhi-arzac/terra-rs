//! Team membership, PvP announcements and team-scoped chat.
use super::{Player, Roster};
use bytes::Bytes;
use terraria_protocol::{Error, NetworkText, Packet};

const COLORS: [[u8; 3]; 6] = [
    [255; 3],
    [218, 59, 59],
    [59, 218, 85],
    [59, 149, 218],
    [242, 221, 100],
    [224, 100, 242],
];

impl Player {
    fn announcement(&self, key: u8) -> Result<Bytes, Error> {
        let name = &self
            .character
            .appearance
            .as_ref()
            .ok_or(Error::Invalid("missing player appearance"))?
            .name;
        Packet::ChatMessage {
            author: 255,
            text: NetworkText::Key(
                format!("LegacyMultiplayer.{key}"),
                vec![NetworkText::Literal(name.clone())],
            ),
            color: COLORS[self.team as usize],
        }
        .encode()
    }
}

impl Roster {
    pub(super) fn change_team(&mut self, slot: u8, team: u8) -> Result<(), Error> {
        let Some(peer) = self.peers.get_mut(&slot) else {
            return Ok(());
        };
        let previous = std::mem::replace(&mut peer.team, team);
        let notice = peer.announcement(if team == 5 { 22 } else { 13 + team })?;
        self.send(
            Some(slot),
            Packet::PlayerTeam { player: slot, team }.encode()?,
        );
        for (&id, peer) in &mut self.peers {
            if id == slot || (peer.team != 0 && (peer.team == previous || peer.team == team)) {
                peer.queue(notice.clone());
            }
        }
        Ok(())
    }

    pub(super) fn change_pvp(&mut self, slot: u8, hostile: bool) -> Result<(), Error> {
        let Some(peer) = self.peers.get_mut(&slot) else {
            return Ok(());
        };
        peer.hostile = hostile;
        let notice = peer.announcement(if hostile { 11 } else { 12 })?;
        self.send(
            Some(slot),
            Packet::PlayerPvp {
                player: slot,
                hostile,
            }
            .encode()?,
        );
        self.send(None, notice);
        Ok(())
    }

    pub(super) fn party_chat(&mut self, slot: u8, text: String) -> Result<(), Error> {
        let Some(sender) = self.peers.get_mut(&slot) else {
            return Ok(());
        };
        let team = sender.team;
        if team == 0 {
            sender.queue(
                Packet::ChatMessage {
                    author: 255,
                    text: NetworkText::key("LegacyMultiplayer.10"),
                    color: [255, 240, 20],
                }
                .encode()?,
            );
        } else if !text.is_empty() {
            let message = Packet::ChatMessage {
                author: slot,
                text: NetworkText::Literal(text),
                color: COLORS[team as usize],
            }
            .encode()?;
            for peer in self.peers.values_mut().filter(|p| p.team == team) {
                peer.queue(message.clone());
            }
        }
        Ok(())
    }
}
