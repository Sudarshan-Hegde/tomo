-- Steam reports where each game's banner lives (newer games use a hashed folder, so the address
-- cannot be guessed from the app id). We store only the part after the app id, and only if it has
-- exactly the expected shape: 40 hex characters + "/header.jpg", or plain "header.jpg".
ALTER TABLE games
    ADD COLUMN banner TEXT CHECK (banner IS NULL OR banner ~ '^([0-9a-f]{40}/)?header\.jpg$');
