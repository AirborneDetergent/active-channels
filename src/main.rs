// This was modified from the !ping: Pong! template and some of the comments
// are mine, but some are from the original template.

use std::collections::HashMap;
use std::env;
use std::time::Duration;

use dotenv::dotenv;

use serenity::async_trait;
use serenity::builder::CreateEmbed;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::{ChannelId, GuildId};
use serenity::prelude::*;
use tokio::sync::RwLockWriteGuard;

// Say this to have the bot create its embed in the channel you say it in
const ACTIVATION_MESSAGE: &str = "DontGuessThisPlease36753653748678";

struct StatesHolder;
impl TypeMapKey for StatesHolder {
	type Value = HashMap<GuildId, ServerState>;
}

// 1 per server
struct ServerState {
	my_msg: Option<Message>,
	channels: HashMap<ChannelId, RecentChannelInfo>,
	config: Config,
}

impl Default for ServerState {
	fn default() -> Self {
		ServerState {
			my_msg: None,
			channels: HashMap::new(),
			config: Config {
				time_limit: 300,
				slots: 8,
			},
		}
	}
}

// There would be a way to change these values if I wasn't lazy
struct Config {
	// How long ago a message can have been sent in a channel for it to
	// still be considered an active channel
	time_limit: i64,
	// The amount of slots to display when the amount of active channels
	// is less than this number. If it goes over, the embed SHOULD just get bigger.
	slots: i32,
}

#[derive(Clone)]
struct RecentChannelInfo {
	unix_timestamp: i64,
	slot: i32,
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
	// Set a handler for the `message` event - so that whenever a new message is received - the
	// closure (or function) passed will be called.
	//
	// Event handlers are dispatched through a threadpool, and so multiple events can be dispatched
	// simultaneously.
	async fn message(&self, ctx: Context, msg: Message) {

		if msg.author.bot {
			return
		}
		let time_limit;
		{
			// Gotta get the state
			let mut state_access = ctx.data.write().await;
			let state = get_state(&mut state_access, msg.guild_id.unwrap());
			time_limit = state.config.time_limit;
			// If the channel already has a place in the embed, use the existing one.
			// Otherwise, get the lowest free one.
			let old_info = state.channels.get(&msg.channel_id);
			// Update the timestamp of the most recent message in the channel
			let info = if let Some(old_data) = old_info {
				RecentChannelInfo {
					unix_timestamp: msg.timestamp.unix_timestamp(),
					slot: old_data.slot
				}
			} else {
				RecentChannelInfo {
					unix_timestamp: msg.timestamp.unix_timestamp(),
					slot: get_free_slot(&state.channels)
				}
			};
			state.channels.insert(msg.channel_id, info);
			update_message(&ctx, state, &msg, 0).await;
			// Activation message. Anyone can use this, so don't let people know what it is.
			if msg.content == ACTIVATION_MESSAGE {
				match msg.channel_id.say(&ctx.http, "Bot setup successful! Make sure to delete the activation message.").await {
					Ok(my_msg) => {
						state.my_msg = Some(my_msg);
					}
					Err(why) => println!("Error sending message: {why:?}"),
				}
			}
		}
		// Wait a little bit longer than the time limit so inactive channels get removed
		tokio::time::sleep(Duration::from_secs(time_limit as u64 + 1)).await;
		{
			// We don't want to hold the state while we're waiting because of the read-write lock
			let mut state_access = ctx.data.write().await;
			let state = get_state(&mut state_access, msg.guild_id.unwrap());
			update_message(&ctx, state, &msg, time_limit + 1).await;
		}
		
	}

	// Set a handler to be called on the `ready` event. This is called when a shard is booted, and
	// a READY payload is sent by Discord. This payload contains data like the current user's guild
	// Ids, current user data, private channels, and more.
	//
	// In this case, just print what the current user's username is.
	async fn ready(&self, _: Context, ready: Ready) {
		println!("{} is connected!", ready.user.name);
	}
}

// Get the state for a guild if it exists, otherwise creates a new one
fn get_state<'a>(state_access: &'a mut RwLockWriteGuard<'_, TypeMap>, id: GuildId) -> &'a mut ServerState {
	let states = state_access.get_mut::<StatesHolder>().unwrap();
	let state = states.get(&id);
	if state.is_some() {
		return states.get_mut(&id).unwrap();
	}
	states.insert(id, ServerState::default());
	return states.get_mut(&id).unwrap();
}

// Generates the text to put in the message with the embed
fn gen_message(state: &mut ServerState, msg: &Message, time_offset: i64) -> String {
	let mut chan_list = Vec::new();
	let mut list_str = String::new();
	// Prune the inactive channels
	state.channels.retain(|id, info| {
		let keep: bool = (msg.timestamp.unix_timestamp() + time_offset) - info.unix_timestamp < state.config.time_limit;
		if keep {
			chan_list.push((id.to_owned(), info.to_owned()));
		}
		keep
	});
	// Put the channels in order based on their slot indices
	chan_list.sort_by_key(|(_, info)| info.slot);
	// Make the list of active channels with each channel going into the right slot
	let mut prev_slot = state.config.slots;
	for (id, info) in chan_list.iter().rev() {
		if prev_slot > 0 {
			let dif = prev_slot - info.slot;
			list_str += "-\n".repeat((dif - 1) as usize).as_str();
		}
		list_str += format!("{}: <t:{}:R>\n", &id.mention().to_string(), info.unix_timestamp).as_str();
		prev_slot = info.slot;
	}
	list_str += "-\n".repeat(prev_slot as usize).as_str();
	list_str
}

// Regenerate the embed message and update it
async fn update_message(ctx: &Context, state: &mut ServerState, msg: &Message, time_offset: i64) {
	let list_str = gen_message(state, msg, time_offset);
	if let Some(my_msg) = state.my_msg.as_mut() {
		if list_str != my_msg.content {
			my_msg.edit(&ctx.http, |edit| {
				let mut embed = CreateEmbed::default();
				embed.title(format!("Showing channels with activity in the past {} seconds\n", state.config.time_limit));
				embed.description(list_str);
				edit.set_embed(embed);
				edit.content("");
				edit
			}).await.unwrap();
		}
	}
}

// Returns the first free slot
fn get_free_slot(channels: &HashMap<ChannelId, RecentChannelInfo>) -> i32 {
	let mut flags = [false; 256];
	for info in channels.values() {
		flags[info.slot as usize] = true;
	}
	flags.iter().position(|x| !x).unwrap() as i32
}

#[tokio::main]
async fn main() {
	dotenv().ok();
	// Configure the client with your Discord bot token in the environment.
	let token = env::var("DISCORD_TOKEN").expect("Expected a token in the environment");
	// Set gateway intents, which decides what events the bot will be notified about
	let intents = GatewayIntents::GUILD_MESSAGES
		| GatewayIntents::MESSAGE_CONTENT;

	// Create a new instance of the Client, logging in as a bot. This will automatically prepend
	// your bot token with "Bot ", which is a requirement by Discord for bot users.
	let mut client =
		Client::builder(&token, intents).event_handler(Handler).await.expect("Err creating client");

	{
		let mut data = client.data.write().await;
		data.insert::<StatesHolder>(HashMap::new());
	}
		
	// Finally, start a single shard, and start listening to events.
	//
	// Shards will automatically attempt to reconnect, and will perform exponential backoff until
	// it reconnects.
	if let Err(why) = client.start().await {
		println!("Client error: {why:?}");
	}
}