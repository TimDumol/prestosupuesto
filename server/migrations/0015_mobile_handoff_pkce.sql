-- Bind custom-scheme handoffs to the app instance that initiated the login.
ALTER TABLE pocket_id_states ADD COLUMN mobile_code_challenge TEXT;
ALTER TABLE pocket_mobile_handoffs ADD COLUMN code_challenge TEXT;
-- Handoffs issued before proof-of-possession was required must not remain usable.
DELETE FROM pocket_mobile_handoffs;
DELETE FROM pocket_id_states WHERE mobile_redirect_uri IS NOT NULL;
