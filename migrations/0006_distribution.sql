CREATE TABLE application_distribution (
    application_id TEXT PRIMARY KEY NOT NULL REFERENCES applications(id) ON DELETE RESTRICT,
    target TEXT NOT NULL CHECK(length(target) BETWEEN 1 AND 64),
    format TEXT NOT NULL CHECK(length(format) BETWEEN 1 AND 32),
    purpose TEXT NOT NULL CHECK(purpose='installer')
);
