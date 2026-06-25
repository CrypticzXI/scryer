CREATE TABLE IF NOT EXISTS user_ui_settings (
    user_id TEXT PRIMARY KEY NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    theme TEXT NOT NULL DEFAULT 'dark',
    date_time_format TEXT NOT NULL DEFAULT 'locale',
    highlight_color TEXT,
    secondary_color TEXT,
    high_contrast_mode BOOLEAN NOT NULL DEFAULT FALSE,
    reduce_motion BOOLEAN NOT NULL DEFAULT FALSE,
    density TEXT NOT NULL DEFAULT 'comfortable',
    sidebar_mode TEXT NOT NULL DEFAULT 'expanded',
    default_landing_view TEXT NOT NULL DEFAULT 'movies',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS user_ui_table_columns (
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    facet TEXT NOT NULL,
    table_view_mode TEXT NOT NULL,
    column_id TEXT NOT NULL,
    column_order INTEGER NOT NULL,
    visible BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, facet, table_view_mode, column_id)
);

CREATE INDEX IF NOT EXISTS idx_user_ui_table_columns_user_view
    ON user_ui_table_columns(user_id, facet, table_view_mode, column_order);
