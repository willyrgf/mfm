ALTER TABLE run_observation_cursors
  DROP CONSTRAINT run_observation_cursors_version_v2,
  DROP COLUMN cursor_kind,
  ADD CONSTRAINT run_observation_cursors_version_v3
    CHECK (cursor_version = 'mfm.run_observation.cursor.v3');
