# Security

## What this program holds

`TelegramToolsData/`, beside the exe, holds your `api_hash` and a Telegram
**session key**. The session key is a bearer credential: anyone who can read
that folder can act as your account, without your phone and without a code.
The folder is restricted to your Windows user when it is created. That
restriction does not exist on a FAT32 or exFAT drive.

`Exports/` is not restricted. Every database there holds a whole chat — every
message, every earlier version, every deleted message it saw, and every media
file. Treat it like the conversation it is.

If you think a session key has leaked, end it from Telegram → **Settings →
Devices**, then press **Sign out** in TelegramTools.

## Reporting a problem

Please do not open a public issue for a security problem. Use GitHub's
**Report a vulnerability** button on this repository's Security tab, which
reaches the maintainer privately.

Never attach `TelegramToolsData/` or a `.sqlite` from `Exports/` to anything.
`tgx.log` holds chat titles and file names, but never message text; look it
over before you share it.
