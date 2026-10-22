-- Word lists of the chat-filter example (lower case).
return {
	-- A message containing one of these is cancelled.
	blocked = { "spam.example", "free-diamonds.example" },
	-- These are replaced by asterisks.
	masked = { "darn", "heck" },
}
