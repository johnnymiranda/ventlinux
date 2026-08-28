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
        ) {
            if let Some(channels) = by_parent.get(&parent) {
                for ch in channels {
                    rows.push((depth, TreeNode::Channel((*ch).clone())));
                    add_users(rows, users_by_ch, ch.id, depth + 1);
                    add_channels(rows, by_parent, users_by_ch, ch.id, depth + 1);
                }
            }
        }
        add_users(&mut rows, &users_by_ch, 0, 0);
        add_channels(&mut rows, &by_parent, &users_by_ch, 0, 0);
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
