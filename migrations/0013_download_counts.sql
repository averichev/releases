-- One atomic counter per immutable published artifact. Release/application totals
-- are derived from these rows, so concurrent requests cannot desynchronize them.
CREATE TABLE artifact_download_counts (
    artifact_id TEXT PRIMARY KEY NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    download_count INTEGER NOT NULL CHECK(typeof(download_count)='integer' AND download_count>=0)
);
