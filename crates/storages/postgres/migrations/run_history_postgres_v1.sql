CREATE TABLE public.mfm_store_schema (
    schema_contract TEXT COLLATE "C" NOT NULL,
    CONSTRAINT mfm_store_schema_pkey PRIMARY KEY (schema_contract),
    CONSTRAINT mfm_store_schema_contract_check
        CHECK (schema_contract = 'mfm.run-history-postgres.v1')
);

INSERT INTO public.mfm_store_schema (schema_contract)
VALUES ('mfm.run-history-postgres.v1');

CREATE TABLE public.mfm_run_frames (
    run_id       TEXT COLLATE "C" NOT NULL,
    run_sequence BIGINT NOT NULL,
    frame_bytes  BYTEA NOT NULL,
    head_digest  TEXT COLLATE "C" NOT NULL,
    CONSTRAINT mfm_run_frames_pkey PRIMARY KEY (run_id, run_sequence),
    CONSTRAINT mfm_run_frames_run_id_check
        CHECK (run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT mfm_run_frames_sequence_check
        CHECK (run_sequence BETWEEN 1 AND 65536),
    CONSTRAINT mfm_run_frames_bytes_check
        CHECK (octet_length(frame_bytes) BETWEEN 1 AND 25231360),
    CONSTRAINT mfm_run_frames_digest_check
        CHECK (head_digest ~ '^content:sha256-v1:[0-9a-f]{64}$')
);

CREATE TABLE public.mfm_run_heads (
    run_id       TEXT COLLATE "C" NOT NULL,
    head_sequence BIGINT NOT NULL,
    total_bytes  BIGINT NOT NULL,
    CONSTRAINT mfm_run_heads_pkey PRIMARY KEY (run_id),
    CONSTRAINT mfm_run_heads_run_id_check
        CHECK (run_id ~ '^run:sha256-jcs-v1:[0-9a-f]{64}$'),
    CONSTRAINT mfm_run_heads_sequence_check
        CHECK (head_sequence BETWEEN 1 AND 65536),
    CONSTRAINT mfm_run_heads_total_bytes_check
        CHECK (total_bytes BETWEEN 1 AND 536870912),
    CONSTRAINT mfm_run_heads_frame_fkey
        FOREIGN KEY (run_id, head_sequence)
        REFERENCES public.mfm_run_frames (run_id, run_sequence)
        ON UPDATE NO ACTION
        ON DELETE NO ACTION
);

REVOKE ALL ON TABLE public.mfm_store_schema FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE public.mfm_run_frames FROM PUBLIC, mfm_runtime;
REVOKE ALL ON TABLE public.mfm_run_heads FROM PUBLIC, mfm_runtime;
GRANT SELECT ON TABLE public.mfm_store_schema TO mfm_runtime;
GRANT SELECT, INSERT ON TABLE public.mfm_run_frames TO mfm_runtime;
GRANT SELECT, INSERT, UPDATE ON TABLE public.mfm_run_heads TO mfm_runtime;
