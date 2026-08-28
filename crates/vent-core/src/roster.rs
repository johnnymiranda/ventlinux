use crate::event::{Channel, CoreEvent, User};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub enum TreeNode {
    Channel(Channel),
    User(User),
}

#[derive(Clone, Debug, Default)]
pub struct Roster {
    pub channels: HashMap<u16, Channel>,
    pub users: HashMap<u16, User>,
    pub talking: HashSet<u16>,
}

impl Roster {
    pub fn apply(&mut self, event: &CoreEvent) {
        match event {
            CoreEvent::ChannelUpserted(ch) => {
                self.channels.insert(ch.id, ch.clone());
            }
            CoreEvent::ChannelRemoved(id) => {
                self.channels.remove(id);
            }
            CoreEvent::UserUpserted(user) => {
                // A user who leaves the channel mid-transmission never sends a
                // talk-end, so drop the talking flag on any channel change.
                if self
                    .users
                    .get(&user.id)
                    .is_some_and(|old| old.channel_id != user.channel_id)
                {
                    self.talking.remove(&user.id);
                }
                self.users.insert(user.id, user.clone());
            }
            CoreEvent::UserRemoved(id) => {
                self.users.remove(id);
                self.talking.remove(id);
            }
            CoreEvent::TalkStarted { user_id, .. } => {
                self.talking.insert(*user_id);
            }
            CoreEvent::TalkEnded { user_id } => {
                self.talking.remove(user_id);
            }
            CoreEvent::Disconnected => self.talking.clear(),
            _ => {}
        }
    }

    /// Depth-first flatten: lobby users, then channels with nested users.
    pub fn flattened_tree(&self) -> Vec<(usize, TreeNode)> {
        let mut by_parent: HashMap<u16, Vec<&Channel>> = HashMap::new();
        for ch in self.channels.values() {
            by_parent.entry(ch.parent).or_default().push(ch);
        }
        for kids in by_parent.values_mut() {
            kids.sort_by(|a, b| a.name.cmp(&b.name));
        }
        let mut users_by_ch: HashMap<u16, Vec<&User>> = HashMap::new();
        for u in self.users.values() {
            users_by_ch.entry(u.channel_id).or_default().push(u);
        }
        for users in users_by_ch.values_mut() {
            users.sort_by(|a, b| a.name.cmp(&b.name));
        }

        let mut rows = Vec::new();
        fn add_users(
            rows: &mut Vec<(usize, TreeNode)>,
            users_by_ch: &HashMap<u16, Vec<&User>>,
            channel_id: u16,
            depth: usize,
        ) {
            if let Some(users) = users_by_ch.get(&channel_id) {
                for user in users {
                    if !user.name.is_empty() {
                        rows.push((depth, TreeNode::User((*user).clone())));
                    }
                }
            }
        }
        fn add_channels(
            rows: &mut Vec<(usize, TreeNode)>,
            by_parent: &HashMap<u16, Vec<&Channel>>,
            users_by_ch: &HashMap<u16, Vec<&User>>,
            parent: u16,
            depth: usize,
            visited: &mut HashSet<u16>,
        ) {
            if let Some(channels) = by_parent.get(&parent) {
                for ch in channels {
                    if !visited.insert(ch.id) {
                        continue;
                    }
                    rows.push((depth, TreeNode::Channel((*ch).clone())));
                    add_users(rows, users_by_ch, ch.id, depth + 1);
                    add_channels(rows, by_parent, users_by_ch, ch.id, depth + 1, visited);
                }
            }
        }
        add_users(&mut rows, &users_by_ch, 0, 0);
        let mut visited = HashSet::new();
        add_channels(&mut rows, &by_parent, &users_by_ch, 0, 0, &mut visited);

        // A transiently missing parent should not hide a channel, and malformed
        // cyclic parent data must not recurse forever. Render each remaining
        // component once at the root.
        let mut remaining: Vec<_> = self
            .channels
            .values()
            .filter(|ch| !visited.contains(&ch.id))
            .collect();
        remaining.sort_by(|a, b| a.name.cmp(&b.name));
        for ch in remaining {
            if !visited.insert(ch.id) {
                continue;
            }
            rows.push((0, TreeNode::Channel(ch.clone())));
            add_users(&mut rows, &users_by_ch, ch.id, 1);
            add_channels(&mut rows, &by_parent, &users_by_ch, ch.id, 1, &mut visited);
        }
        rows
    }

    pub fn channel_name(&self, id: u16) -> String {
        if id == 0 {
            "(lobby)".into()
        } else {
            self.channels
                .get(&id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("#{id}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel(id: u16, parent: u16, name: &str) -> Channel {
        Channel {
            id,
            parent,
            name: name.into(),
            phonetic: String::new(),
            comment: String::new(),
            password_protected: false,
            codec: 0,
            codec_format: 0,
        }
    }

    fn user(id: u16, channel_id: u16, name: &str) -> User {
        User {
            id,
            channel_id,
            name: name.into(),
            phonetic: String::new(),
            comment: String::new(),
            url: String::new(),
            rank_id: 0,
            guest: false,
            global_mute: false,
            channel_mute: false,
            phantom: false,
            accepts_pages: true,
            accepts_private_chat: true,
        }
    }

    #[test]
    fn changing_channel_stops_showing_a_user_as_talking() {
        let mut roster = Roster::default();
        roster.apply(&CoreEvent::UserUpserted(user(5, 1, "Ann")));
        roster.apply(&CoreEvent::TalkStarted {
            user_id: 5,
            rate: 48_000,
        });
        assert!(roster.talking.contains(&5));

        // Leaving the channel mid-transmission produces no talk-end.
        roster.apply(&CoreEvent::UserUpserted(user(5, 2, "Ann")));

        assert!(!roster.talking.contains(&5));
    }

    #[test]
    fn a_plain_user_update_keeps_the_talking_flag() {
        let mut roster = Roster::default();
        roster.apply(&CoreEvent::UserUpserted(user(5, 1, "Ann")));
        roster.apply(&CoreEvent::TalkStarted {
            user_id: 5,
            rate: 48_000,
        });

        let mut renamed = user(5, 1, "Ann");
        renamed.comment = "brb".into();
        roster.apply(&CoreEvent::UserUpserted(renamed));

        assert!(roster.talking.contains(&5));
    }

    #[test]
    fn flattened_tree_includes_orphaned_channels() {
        let mut roster = Roster::default();
        roster.channels.insert(2, channel(2, 99, "Orphan"));

        let rows = roster.flattened_tree();

        assert!(matches!(
            rows.as_slice(),
            [(0, TreeNode::Channel(ch))] if ch.id == 2
        ));
    }

    #[test]
    fn flattened_tree_handles_parent_cycles_once() {
        let mut roster = Roster::default();
        roster.channels.insert(1, channel(1, 2, "A"));
        roster.channels.insert(2, channel(2, 1, "B"));

        let ids: Vec<_> = roster
            .flattened_tree()
            .into_iter()
            .filter_map(|(_, node)| match node {
                TreeNode::Channel(ch) => Some(ch.id),
                TreeNode::User(_) => None,
            })
            .collect();

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&1));
        assert!(ids.contains(&2));
    }
}
