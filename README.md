# TG Archive Bot

This is the bot meant for archiving messages in TeenGovernment and related servers.

The bot archives new messages and edits, including attachments and embeds. Messages stay in the archive if they are deleted from Discord.

An image is available at git.ewenlau.net/ewenlau/tg-archive-bot:latest for the stable version and git.ewenlau.net/ewenlau/tg-archive-bot:dev for the development (or testing) version.

The web archive uses Discord OAuth2 with the `identify` scope. Add `/login/discord/callback` as a redirect in the Discord developer portal, then set `TG_BOT_DISCORD_CLIENT_ID`, `TG_BOT_DISCORD_CLIENT_SECRET`, and `TG_BOT_DISCORD_REDIRECT_URI`.
