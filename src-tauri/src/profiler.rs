use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};

use crate::db::{self, ConnectionConfig, SqlClient};

const XE_POLL_PAGE_SIZE: usize = 5000;
const XE_POLL_MAX_DRAIN_PASSES: usize = 8;
const XE_FILE_MAX_SIZE_MB: usize = 50;
const XE_FILE_MAX_ROLLOVER_FILES: usize = 32;
const MIN_TIMESTAMP: &str = "1900-01-01T00:00:00.000";

const XE_SESSION_NAME: &str = "SimpleSQLProfilerXE";
const XE_SELF_FILTER: &str = "([sqlserver].[client_app_name] NOT LIKE N''SimpleSQLProfiler%'') AND ([sqlserver].[client_app_name] NOT LIKE N''SimpleSQLQueryWindow%'')";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStorageMode {
    InMemory,
    Files,
}

impl Default for CaptureStorageMode {
    fn default() -> Self {
        Self::Files
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CaptureOptions {
    #[serde(default, alias = "storageMode")]
    pub storage_mode: CaptureStorageMode,
}

fn build_event_definition(event_name: &str) -> String {
    format!(
        "ADD EVENT {event_name}(\n    ACTION(\n        package0.event_sequence,\n        sqlserver.session_id,\n        sqlserver.client_app_name,\n        sqlserver.client_hostname,\n        sqlserver.server_principal_name,\n        sqlserver.database_name,\n        sqlserver.database_id,\n        sqlserver.sql_text\n    )\n    WHERE {XE_SELF_FILTER}\n)"
    )
}

fn build_event_definitions() -> String {
    vec![
        build_event_definition("sqlserver.rpc_completed"),
        build_event_definition("sqlserver.sql_batch_completed"),
        build_event_definition("sqlserver.exec_prepared_sql"),
        build_event_definition("sqlserver.sp_statement_completed"),
        build_event_definition("sqlserver.sql_statement_completed"),
        build_event_definition("sqlserver.rpc_starting"),
        build_event_definition("sqlserver.sql_batch_starting"),
        build_event_definition("sqlserver.sp_statement_starting"),
        build_event_definition("sqlserver.sql_statement_starting"),
        build_event_definition("sqlserver.module_start"),
        build_event_definition("sqlserver.module_end"),
        build_event_definition("sqlserver.prepare_sql"),
        build_event_definition("sqlserver.unprepare_sql"),
    ]
    .join(",\n")
}

fn build_target_definition(storage_mode: CaptureStorageMode) -> String {
    match storage_mode {
        CaptureStorageMode::InMemory => "
ADD TARGET package0.ring_buffer(
    SET max_memory = 4096,
        max_events_limit = 5000
)"
        .to_string(),
        CaptureStorageMode::Files => format!(
            "
ADD TARGET package0.event_file(
    SET filename = N''' + REPLACE(@base_file, N'''', N'''''') + N''',
        max_file_size = ({XE_FILE_MAX_SIZE_MB}),
        max_rollover_files = ({XE_FILE_MAX_ROLLOVER_FILES})
)"
        ),
    }
}

fn build_session_options(storage_mode: CaptureStorageMode) -> &'static str {
    match storage_mode {
        CaptureStorageMode::InMemory => {
            "MAX_MEMORY = 8 MB,
    EVENT_RETENTION_MODE = ALLOW_SINGLE_EVENT_LOSS,
    MAX_DISPATCH_LATENCY = 1 SECONDS,
    TRACK_CAUSALITY = OFF,
    STARTUP_STATE = OFF"
        }
        CaptureStorageMode::Files => {
            "MAX_MEMORY = 16 MB,
    EVENT_RETENTION_MODE = NO_EVENT_LOSS,
    MAX_DISPATCH_LATENCY = 1 SECONDS,
    TRACK_CAUSALITY = OFF,
    STARTUP_STATE = OFF"
        }
    }
}

fn build_create_and_start_sql(storage_mode: CaptureStorageMode) -> String {
    let event_definitions = build_event_definitions();
    let target_definition = build_target_definition(storage_mode);
    let session_options = build_session_options(storage_mode);
    let storage_setup = if matches!(storage_mode, CaptureStorageMode::Files) {
        "
SET @log_dir = CONVERT(nvarchar(4000), SERVERPROPERTY('InstanceDefaultLogPath'));

IF @log_dir IS NULL OR LEN(LTRIM(RTRIM(@log_dir))) = 0
BEGIN
    SET @errorlog = CONVERT(nvarchar(4000), SERVERPROPERTY('ErrorLogFileName'));
    IF @errorlog IS NOT NULL AND LEN(@errorlog) > 0
    BEGIN
        SET @log_dir = LEFT(
            @errorlog,
            LEN(@errorlog) - CHARINDEX(@path_separator, REVERSE(@errorlog))
        );
    END
END

IF @log_dir IS NULL OR LEN(LTRIM(RTRIM(@log_dir))) = 0
BEGIN
    THROW 50001, N'Unable to resolve a writable SQL Server log directory for Extended Events trace files.', 1;
END

IF RIGHT(@log_dir, LEN(@path_separator)) <> @path_separator
BEGIN
    SET @log_dir = @log_dir + @path_separator;
END

SET @file_token = REPLACE(CONVERT(nvarchar(36), NEWID()), N'-', N'');
SET @base_file = @log_dir + @session_name + N'_' + @file_token + N'.xel';
SET @file_pattern = LEFT(@base_file, LEN(@base_file) - 4) + N'*.xel';
"
    } else {
        ""
    };

    format!(
        "
DECLARE @session_name sysname = @P1;
DECLARE @sql nvarchar(max);
DECLARE @path_separator nvarchar(4);
DECLARE @errorlog nvarchar(4000);
DECLARE @log_dir nvarchar(4000);
DECLARE @file_token nvarchar(32);
DECLARE @base_file nvarchar(4000);
DECLARE @file_pattern nvarchar(4000);

IF EXISTS (SELECT 1 FROM sys.server_event_sessions WHERE name = @session_name)
BEGIN
    BEGIN TRY
        SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER STATE = STOP;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;

    BEGIN TRY
        SET @sql = N'DROP EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;
END

SET @path_separator = COALESCE(CONVERT(nvarchar(4), SERVERPROPERTY('PathSeparator')), N'\\');

{storage_setup}

SET @sql = N'
CREATE EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER
{event_definitions}
{target_definition}
WITH (
    {session_options}
);';
EXEC(@sql);

SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER STATE = START;';
EXEC(@sql);

SELECT @session_name AS session_name, @base_file AS base_file, @file_pattern AS file_pattern;
"
    )
}

fn build_stop_and_drop_sql() -> String {
    "
DECLARE @session_name sysname = @P1;
DECLARE @sql nvarchar(max);

IF EXISTS (SELECT 1 FROM sys.server_event_sessions WHERE name = @session_name)
BEGIN
    BEGIN TRY
        SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER STATE = STOP;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;

    BEGIN TRY
        SET @sql = N'DROP EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;
END
"
    .to_string()
}

fn build_stop_sql() -> String {
    "
DECLARE @session_name sysname = @P1;
DECLARE @sql nvarchar(max);

IF EXISTS (SELECT 1 FROM sys.server_event_sessions WHERE name = @session_name)
BEGIN
    BEGIN TRY
        SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER STATE = STOP;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;
END
"
    .to_string()
}

const XE_POLL_EVENT_FILE: &str = r#"
WITH raw AS (
    SELECT TOP (5000) WITH TIES
        CAST(event_data AS xml) AS event_data_xml,
        CAST(file_name AS nvarchar(4000)) AS source_file_name,
        CAST(file_offset AS bigint) AS source_file_offset
    FROM sys.fn_xe_file_target_read_file(@P1, NULL, @P2, @P3)
    ORDER BY file_name, file_offset
),
parsed AS (
    SELECT
        r.source_file_name,
        r.source_file_offset,
        node.value('@name', 'nvarchar(128)') AS event_name,
        TRY_CONVERT(datetimeoffset(7), node.value('@timestamp', 'nvarchar(50)')) AS start_time_utc,
        ISNULL(node.value('(action[@name="event_sequence"]/value)[1]', 'bigint'), 0) AS event_sequence,
        ISNULL(node.value('(action[@name="session_id"]/value)[1]', 'int'), 0) AS session_id,
        ISNULL(node.value('(data[@name="duration"]/value)[1]', 'bigint'), 0) AS duration_us,
        ISNULL(node.value('(data[@name="cpu_time"]/value)[1]', 'bigint'), 0) AS cpu_time_us,
        ISNULL(node.value('(data[@name="logical_reads"]/value)[1]', 'bigint'), 0) AS logical_reads,
        ISNULL(node.value('(data[@name="physical_reads"]/value)[1]', 'bigint'), 0) AS physical_reads,
        ISNULL(node.value('(data[@name="writes"]/value)[1]', 'bigint'), 0) AS writes,
        ISNULL(node.value('(data[@name="row_count"]/value)[1]', 'bigint'), 0) AS row_count,
        CAST(ISNULL(node.value('(data[@name="statement"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS statement_text,
        CAST(ISNULL(node.value('(data[@name="batch_text"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS batch_text,
        CAST(ISNULL(node.value('(data[@name="options_text"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS options_text,
        CAST(ISNULL(node.value('(action[@name="sql_text"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS sql_text_action,
        CAST(ISNULL(node.value('(data[@name="prepared_statement_text"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS prepared_statement_text,
        CAST(ISNULL(node.value('(data[@name="object_name"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS object_name_data,
        CAST(ISNULL(node.value('(action[@name="database_name"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS database_name_action,
        ISNULL(node.value('(action[@name="database_id"]/value)[1]', 'int'), 0) AS database_id_action,
        CAST(ISNULL(node.value('(data[@name="database_name"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS database_name_data,
        ISNULL(node.value('(data[@name="database_id"]/value)[1]', 'int'), 0) AS database_id_data,
        CAST(ISNULL(node.value('(action[@name="server_principal_name"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS login_name,
        CAST(ISNULL(node.value('(action[@name="client_hostname"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS host_name,
        CAST(ISNULL(node.value('(action[@name="client_app_name"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS program_name
    FROM raw r
    CROSS APPLY r.event_data_xml.nodes('/event') AS n(node)
)
SELECT
    source_file_name AS file_name,
    source_file_offset AS file_offset,
    event_name,
    CONVERT(varchar(27), CAST(start_time_utc AS datetime2(3)), 126) AS start_time,
    event_sequence,
    duration_us,
    cpu_time_us,
    logical_reads,
    physical_reads,
    writes,
    row_count,
    CASE
        WHEN event_name IN (
            N'rpc_starting',
            N'rpc_completed',
            N'sp_statement_starting',
            N'sp_statement_completed',
            N'sql_statement_starting',
            N'sql_statement_completed'
        ) AND LEN(statement_text) > 0 THEN
            CASE
                WHEN event_name IN (N'rpc_starting', N'rpc_completed')
                     AND LEN(sql_text_action) > LEN(statement_text)
                THEN sql_text_action
                ELSE statement_text
            END
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'module_start', N'module_end')
             AND LEN(object_name_data) > 0 THEN object_name_data
        WHEN event_name = N'prepare_sql' AND LEN(prepared_statement_text) > 0 THEN prepared_statement_text
        WHEN event_name = N'exec_prepared_sql' AND LEN(sql_text_action) > 0 THEN sql_text_action
        WHEN LEN(batch_text) > 0 THEN batch_text
        WHEN LEN(options_text) > 0 THEN options_text
        ELSE sql_text_action
    END AS sql_text,
    CASE
        WHEN event_name IN (
            N'rpc_starting',
            N'rpc_completed',
            N'sp_statement_starting',
            N'sp_statement_completed',
            N'sql_statement_starting',
            N'sql_statement_completed'
        ) THEN statement_text
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'module_start', N'module_end')
             THEN object_name_data
        WHEN event_name = N'prepare_sql' THEN prepared_statement_text
        WHEN event_name = N'exec_prepared_sql' THEN sql_text_action
        ELSE N''
    END AS current_statement,
    CASE
        WHEN LEN(database_name_action) > 0 THEN database_name_action
        WHEN LEN(database_name_data) > 0 THEN database_name_data
        WHEN database_id_action > 0 THEN ISNULL(DB_NAME(database_id_action), N'')
        WHEN database_id_data > 0 THEN ISNULL(DB_NAME(database_id_data), N'')
        ELSE N''
    END AS database_name,
    CASE
        WHEN database_id_action > 0 THEN database_id_action
        WHEN database_id_data > 0 THEN database_id_data
        ELSE 0
    END AS database_id_value,
    login_name,
    host_name,
    program_name,
    session_id
FROM parsed
WHERE event_name IN (
    N'rpc_starting',
    N'rpc_completed',
    N'sql_batch_starting',
    N'sql_batch_completed',
    N'sp_statement_starting',
    N'sp_statement_completed',
    N'sql_statement_starting',
    N'sql_statement_completed',
    N'exec_prepared_sql',
    N'prepare_sql',
    N'unprepare_sql',
    N'module_start',
    N'module_end'
)
  AND start_time_utc IS NOT NULL
ORDER BY file_name ASC, file_offset ASC, event_sequence ASC;
"#;

const XE_POLL_EVENTS: &str = "
WITH xe_data AS (
    SELECT CAST(st.target_data AS xml) AS target_data
    FROM sys.dm_xe_sessions s
    INNER JOIN sys.dm_xe_session_targets st
        ON st.event_session_address = s.address
    WHERE s.name = @P1
      AND st.target_name = N'ring_buffer'
),
parsed AS (
    SELECT
        node.value('@name', 'nvarchar(128)') AS event_name,
        TRY_CONVERT(datetimeoffset(7), node.value('@timestamp', 'nvarchar(50)')) AS start_time_utc,
        ISNULL(node.value('(action[@name=\"event_sequence\"]/value)[1]', 'bigint'), 0) AS event_sequence,
        ISNULL(node.value('(action[@name=\"session_id\"]/value)[1]', 'int'), 0) AS session_id,
        ISNULL(node.value('(data[@name=\"duration\"]/value)[1]', 'bigint'), 0) AS duration_us,
        ISNULL(node.value('(data[@name=\"cpu_time\"]/value)[1]', 'bigint'), 0) AS cpu_time_us,
        ISNULL(node.value('(data[@name=\"logical_reads\"]/value)[1]', 'bigint'), 0) AS logical_reads,
        ISNULL(node.value('(data[@name=\"physical_reads\"]/value)[1]', 'bigint'), 0) AS physical_reads,
        ISNULL(node.value('(data[@name=\"writes\"]/value)[1]', 'bigint'), 0) AS writes,
        ISNULL(node.value('(data[@name=\"row_count\"]/value)[1]', 'bigint'), 0) AS row_count,
        CAST(ISNULL(node.value('(data[@name=\"statement\"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS statement_text,
        CAST(ISNULL(node.value('(data[@name=\"batch_text\"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS batch_text,
        CAST(ISNULL(node.value('(data[@name=\"options_text\"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS options_text,
        CAST(ISNULL(node.value('(action[@name=\"sql_text\"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS sql_text_action,
        CAST(ISNULL(node.value('(data[@name=\"prepared_statement_text\"]/value)[1]', 'nvarchar(4000)'), N'') AS nvarchar(4000)) AS prepared_statement_text,
        CAST(ISNULL(node.value('(data[@name=\"object_name\"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS object_name_data,
        CAST(ISNULL(node.value('(action[@name=\"database_name\"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS database_name_action,
        ISNULL(node.value('(action[@name=\"database_id\"]/value)[1]', 'int'), 0) AS database_id_action,
        CAST(ISNULL(node.value('(data[@name=\"database_name\"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS database_name_data,
        ISNULL(node.value('(data[@name=\"database_id\"]/value)[1]', 'int'), 0) AS database_id_data,
        CAST(ISNULL(node.value('(action[@name=\"server_principal_name\"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS login_name,
        CAST(ISNULL(node.value('(action[@name=\"client_hostname\"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS host_name,
        CAST(ISNULL(node.value('(action[@name=\"client_app_name\"]/value)[1]', 'nvarchar(128)'), N'') AS nvarchar(128)) AS program_name
    FROM xe_data d
    CROSS APPLY d.target_data.nodes('/RingBufferTarget/event') AS n(node)
)
SELECT TOP (5000)
    event_name,
    CONVERT(varchar(27), CAST(start_time_utc AS datetime2(3)), 126) AS start_time,
    CONVERT(varchar(27), CAST(start_time_utc AS datetime2(7)), 126) AS cursor_time,
    event_sequence,
    duration_us,
    cpu_time_us,
    logical_reads,
    physical_reads,
    writes,
    row_count,
    CASE
        WHEN event_name IN (
            N'rpc_starting',
            N'rpc_completed',
            N'sp_statement_starting',
            N'sp_statement_completed',
            N'sql_statement_starting',
            N'sql_statement_completed'
        )
             AND LEN(statement_text) > 0 THEN
            CASE
                WHEN event_name IN (N'rpc_starting', N'rpc_completed')
                     AND LEN(sql_text_action) > LEN(statement_text)
                THEN sql_text_action
                ELSE statement_text
            END
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'module_start', N'module_end')
             AND LEN(object_name_data) > 0 THEN object_name_data
        WHEN event_name = N'prepare_sql' AND LEN(prepared_statement_text) > 0 THEN prepared_statement_text
        WHEN event_name = N'exec_prepared_sql' AND LEN(sql_text_action) > 0 THEN sql_text_action
        WHEN LEN(batch_text) > 0 THEN batch_text
        WHEN LEN(options_text) > 0 THEN options_text
        ELSE sql_text_action
    END AS sql_text,
    CASE
        WHEN event_name IN (
            N'rpc_starting',
            N'rpc_completed',
            N'sp_statement_starting',
            N'sp_statement_completed',
            N'sql_statement_starting',
            N'sql_statement_completed'
        )
            THEN statement_text
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'module_start', N'module_end')
             THEN object_name_data
        WHEN event_name = N'prepare_sql' THEN prepared_statement_text
        WHEN event_name = N'exec_prepared_sql' THEN sql_text_action
        ELSE N''
    END AS current_statement,
    CASE
        WHEN LEN(database_name_action) > 0 THEN database_name_action
        WHEN LEN(database_name_data) > 0 THEN database_name_data
        WHEN database_id_action > 0 THEN ISNULL(DB_NAME(database_id_action), N'')
        WHEN database_id_data > 0 THEN ISNULL(DB_NAME(database_id_data), N'')
        ELSE N''
    END AS database_name,
    CASE
        WHEN database_id_action > 0 THEN database_id_action
        WHEN database_id_data > 0 THEN database_id_data
        ELSE 0
    END AS database_id_value,
    login_name,
    host_name,
    program_name,
    session_id
FROM parsed
WHERE event_name IN (
    N'rpc_starting',
    N'rpc_completed',
    N'sql_batch_starting',
    N'sql_batch_completed',
    N'sp_statement_starting',
    N'sp_statement_completed',
    N'sql_statement_starting',
    N'sql_statement_completed',
    N'exec_prepared_sql',
    N'prepare_sql',
    N'unprepare_sql',
    N'module_start',
    N'module_end'
)
  AND start_time_utc IS NOT NULL
  AND (
      CAST(start_time_utc AS datetime2(7)) > TRY_CONVERT(datetime2(7), @P2)
      OR (
          CAST(start_time_utc AS datetime2(7)) = TRY_CONVERT(datetime2(7), @P2)
          AND event_sequence > @P3
      )
  )
ORDER BY
    CAST(start_time_utc AS datetime2(7)) ASC,
    event_sequence ASC;
";

#[derive(Debug, Clone, Serialize)]
pub struct QueryEvent {
    pub id: String,
    pub session_id: i32,
    pub start_time: String,
    pub event_name: String,
    pub database_name: String,
    pub cpu_time: i32,
    pub elapsed_time: i32,
    pub physical_reads: i64,
    pub writes: i64,
    pub logical_reads: i64,
    pub row_count: i64,
    pub sql_text: String,
    pub current_statement: String,
    pub login_name: String,
    pub host_name: String,
    pub program_name: String,
    pub captured_at: String,
    pub event_status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfilerStatus {
    pub connected: bool,
    pub capturing: bool,
    pub error: Option<String>,
    pub note: Option<String>,
    pub toast: Option<String>,
}

#[derive(Debug, Clone)]
struct PolledEvent {
    event: QueryEvent,
    cursor_time: String,
    event_sequence: i64,
    file_name: String,
    file_offset: i64,
}

#[derive(Debug, Clone)]
struct SessionTargetInfo {
    file_pattern: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct StopSessionFeedback {
    note: Option<String>,
    toast: Option<String>,
}

#[derive(Debug, Clone)]
struct SessionReadBookmark {
    file_name: String,
    file_offset: i64,
}

#[derive(Debug, Clone)]
struct ActiveSession {
    session_name: String,
    storage_mode: CaptureStorageMode,
    file_pattern: Option<String>,
    read_only: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueryResultData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

pub enum ProfilerCommand {
    Connect {
        config: ConnectionConfig,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Disconnect {
        reply: oneshot::Sender<Result<(), String>>,
    },
    StartCapture {
        options: CaptureOptions,
        reply: oneshot::Sender<Result<(), String>>,
    },
    StopCapture {
        reply: oneshot::Sender<Result<(), String>>,
    },
    ExecuteQuery {
        sql: String,
        reply: oneshot::Sender<Result<QueryResultData, String>>,
    },
}

pub fn spawn_profiler_task(app: tauri::AppHandle) -> mpsc::Sender<ProfilerCommand> {
    let (tx, rx) = mpsc::channel::<ProfilerCommand>(32);

    tauri::async_runtime::spawn(profiler_loop(rx, app));

    tx
}

async fn profiler_loop(mut rx: mpsc::Receiver<ProfilerCommand>, app: tauri::AppHandle) {
    use tauri::Emitter;

    let mut control_client: Option<SqlClient> = None;
    let mut active_config: Option<ConnectionConfig> = None;
    let mut active_session: Option<ActiveSession> = None;
    let mut polling_task: Option<tauri::async_runtime::JoinHandle<()>> = None;
    let mut poll_run_flag: Option<Arc<AtomicBool>> = None;

    fn emit_status(
        app: &tauri::AppHandle,
        connected: bool,
        capturing: bool,
        error: Option<String>,
        note: Option<String>,
        toast: Option<String>,
    ) {
        let _ = app.emit(
            "profiler-status",
            ProfilerStatus {
                connected,
                capturing,
                error,
                note,
                toast,
            },
        );
    }

    while let Some(cmd) = rx.recv().await {
        match cmd {
            ProfilerCommand::Connect { config, reply } => {
                if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_session(c, session).await;
                }
                stop_polling_gracefully(&mut poll_run_flag, &mut polling_task).await;
                if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_and_close_session(c, session).await;
                }
                active_session = None;

                match db::connect(&config).await {
                    Ok(c) => {
                        control_client = Some(c);
                        active_config = Some(config);
                        emit_status(&app, true, false, None, None, None);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        control_client = None;
                        active_config = None;
                        emit_status(&app, false, false, Some(e.clone()), None, None);
                        let _ = reply.send(Err(e));
                    }
                }
            }
            ProfilerCommand::Disconnect { reply } => {
                if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_session(c, session).await;
                }
                stop_polling_gracefully(&mut poll_run_flag, &mut polling_task).await;

                let (stop_error, stop_note, stop_toast) = if let (Some(c), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    match stop_and_close_session(c, session).await {
                        Ok(feedback) => (None, feedback.note, feedback.toast),
                        Err(error) => (Some(error), None, None),
                    }
                } else {
                    (None, None, None)
                };
                control_client = None;
                active_config = None;
                active_session = None;
                emit_status(&app, false, false, stop_error, stop_note, stop_toast);
                let _ = reply.send(Ok(()));
            }
            ProfilerCommand::StartCapture { options, reply } => {
                if control_client.is_none() {
                    let _ = reply.send(Err("Not connected".into()));
                    continue;
                }

                if let (Some(control), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_session(control, session).await;
                }
                stop_polling_gracefully(&mut poll_run_flag, &mut polling_task).await;
                if let (Some(control), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_and_close_session(control, session).await;
                    active_session = None;
                }

                let session = match control_client.as_mut() {
                    Some(control) => match start_session(control, XE_SESSION_NAME, options).await {
                        Ok(session) => session,
                        Err(e) => {
                            let _ = reply.send(Err(e));
                            continue;
                        }
                    },
                    None => {
                        let _ = reply.send(Err("Not connected".into()));
                        continue;
                    }
                };
                active_session = Some(session.clone());

                let Some(cfg) = active_config.clone() else {
                    let _ = reply.send(Err("Missing connection configuration".into()));
                    continue;
                };

                match db::connect(&cfg).await {
                    Ok(poll_client) => {
                        let run_flag = Arc::new(AtomicBool::new(true));
                        poll_run_flag = Some(run_flag.clone());
                        polling_task = Some(spawn_polling_task(
                            app.clone(),
                            poll_client,
                            session.clone(),
                            run_flag,
                        ));
                        let capture_note = if session.read_only {
                            Some(match session.storage_mode {
                                CaptureStorageMode::Files => {
                                    "Attached to an existing server XE trace-file session in read-only mode. Stop will only stop local polling.".to_string()
                                }
                                CaptureStorageMode::InMemory => {
                                    "Attached to an existing server XE in-memory session in read-only mode. Stop will only stop local polling.".to_string()
                                }
                            })
                        } else {
                            None
                        };
                        emit_status(&app, true, true, None, capture_note, None);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        if let (Some(control), Some(s)) =
                            (control_client.as_mut(), active_session.as_ref())
                        {
                            let _ = stop_and_close_session(control, s).await;
                        }
                        active_session = None;
                        let message = format!("Failed to start polling stream: {e}");
                        emit_status(&app, true, false, Some(message.clone()), None, None);
                        let _ = reply.send(Err(message));
                    }
                }
            }
            ProfilerCommand::StopCapture { reply } => {
                if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_session(c, session).await;
                }
                stop_polling_gracefully(&mut poll_run_flag, &mut polling_task).await;
                let stop_result = if let (Some(c), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    stop_and_close_session(c, session).await
                } else {
                    Ok(StopSessionFeedback::default())
                };
                active_session = None;

                match stop_result {
                    Ok(feedback) => {
                        emit_status(
                            &app,
                            control_client.is_some(),
                            false,
                            None,
                            feedback.note,
                            feedback.toast,
                        );
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        emit_status(
                            &app,
                            control_client.is_some(),
                            false,
                            Some(e.clone()),
                            None,
                            None,
                        );
                        let _ = reply.send(Err(e));
                    }
                }
            }
            ProfilerCommand::ExecuteQuery { sql, reply } => {
                let Some(cfg) = active_config.clone() else {
                    let _ = reply.send(Err("Not connected".into()));
                    continue;
                };
                let result = execute_user_query(&cfg, &sql).await;
                let _ = reply.send(result);
            }
        }
    }

    if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref()) {
        let _ = stop_session(c, session).await;
    }
    stop_polling_gracefully(&mut poll_run_flag, &mut polling_task).await;
    if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref()) {
        let _ = stop_and_close_session(c, session).await;
    }
}

fn spawn_polling_task(
    app: tauri::AppHandle,
    mut poll_client: SqlClient,
    session: ActiveSession,
    run_flag: Arc<AtomicBool>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        use tauri::Emitter;

        let mut bookmark: Option<SessionReadBookmark> = None;
        let mut last_timestamp = String::from(MIN_TIMESTAMP);
        let mut last_event_sequence = -1_i64;
        let mut seen_without_sequence_at_timestamp = HashSet::<String>::new();
        let mut transient_failures = 0_u32;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            let shutdown_requested = !run_flag.load(Ordering::Acquire);

            let mut pass = 0usize;
            loop {
                let events = match poll_session_events(
                    &mut poll_client,
                    &session,
                    bookmark.as_ref(),
                    &last_timestamp,
                    last_event_sequence,
                )
                .await
                {
                    Ok(events) => events,
                    Err(e) => {
                        if is_transient_session_poll_error(&e) {
                            transient_failures = transient_failures.saturating_add(1);
                            if transient_failures <= 10 {
                                // Back off to next interval tick instead of busy-looping.
                                break;
                            }
                            let _ = app.emit(
                                "profiler-status",
                                ProfilerStatus {
                                    connected: true,
                                    capturing: false,
                                    error: Some(format!(
                                        "Extended Events session is unavailable after repeated retries: {e}"
                                    )),
                                    note: None,
                                    toast: None,
                                },
                            );
                            return;
                        }
                        let _ = app.emit(
                            "profiler-status",
                            ProfilerStatus {
                                connected: true,
                                capturing: false,
                                error: Some(e),
                                note: None,
                                toast: None,
                            },
                        );
                        return;
                    }
                };
                transient_failures = 0;

                if events.is_empty() {
                    break;
                }

                let event_count = events.len();
                let now = chrono::Utc::now().to_rfc3339();
                let next_bookmark = if matches!(session.storage_mode, CaptureStorageMode::Files) {
                    events.last().map(|event| SessionReadBookmark {
                        file_name: event.file_name.clone(),
                        file_offset: event.file_offset,
                    })
                } else {
                    None
                };
                for mut polled in events {
                    if !run_flag.load(Ordering::Acquire) {
                        break;
                    }
                    if matches!(session.storage_mode, CaptureStorageMode::InMemory) {
                        let ts = polled.cursor_time.clone();
                        let seq = polled.event_sequence;
                        if ts < last_timestamp {
                            continue;
                        }

                        if ts > last_timestamp {
                            last_timestamp = ts.clone();
                            last_event_sequence = -1;
                            seen_without_sequence_at_timestamp.clear();
                        }

                        if seq > 0 {
                            if seq <= last_event_sequence {
                                continue;
                            }
                            last_event_sequence = seq;
                        } else {
                            let fallback_key = format!(
                                "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                                polled.event.event_name,
                                polled.event.session_id,
                                polled.event.elapsed_time,
                                polled.event.cpu_time,
                                polled.event.logical_reads,
                                polled.event.physical_reads,
                                polled.event.writes,
                                polled.event.row_count,
                                polled.event.database_name,
                                polled.event.sql_text
                            );
                            if !seen_without_sequence_at_timestamp.insert(fallback_key) {
                                continue;
                            }
                            if last_event_sequence < 0 {
                                last_event_sequence = 0;
                            }
                        }
                    }
                    polled.event.id = uuid::Uuid::new_v4().to_string();
                    polled.event.captured_at = now.clone();
                    polled.event.event_status =
                        derive_event_status(&polled.event.event_name).to_string();
                    let _ = app.emit("query-event", &polled.event);
                }
                if let Some(next_bookmark) = next_bookmark {
                    bookmark = Some(next_bookmark);
                }

                pass += 1;
                if event_count < XE_POLL_PAGE_SIZE || pass >= XE_POLL_MAX_DRAIN_PASSES {
                    break;
                }
            }

            if shutdown_requested {
                break;
            }
        }
    })
}

async fn start_session(
    client: &mut SqlClient,
    session_name: &str,
    options: CaptureOptions,
) -> Result<ActiveSession, String> {
    match start_server_scoped_session(client, session_name, options.storage_mode).await {
        Ok(target_info) => Ok(ActiveSession {
            session_name: session_name.to_string(),
            storage_mode: options.storage_mode,
            file_pattern: target_info.file_pattern,
            read_only: false,
        }),
        Err(server_error) => {
            if is_permission_error(&server_error) {
                if matches!(server_session_exists(client, session_name).await, Ok(true)) {
                    match options.storage_mode {
                        CaptureStorageMode::Files => {
                            match server_session_get_event_file_pattern(client, session_name).await
                            {
                                Ok(Some(file_pattern)) => {
                                    return Ok(ActiveSession {
                                        session_name: session_name.to_string(),
                                        storage_mode: options.storage_mode,
                                        file_pattern: Some(file_pattern),
                                        read_only: true,
                                    });
                                }
                                Ok(None) => {
                                    return Err(format!(
                                        "Trace files mode requires a server-scoped Extended Events session with an event_file target.\n\
Failed to create/start server-scoped session: {server_error}\n\
An existing session named '{session_name}' is running, but it does not expose an event_file target that this app can read.\n\
Grant ALTER ANY EVENT SESSION plus VIEW SERVER STATE (or VIEW SERVER PERFORMANCE STATE on SQL Server 2022+), or have a DBA create the server-scoped session with an event_file target."
                                    ));
                                }
                                Err(attach_error) => {
                                    return Err(format!(
                                        "Trace files mode requires a server-scoped Extended Events session with an event_file target.\n\
Failed to create/start server-scoped session: {server_error}\n\
An existing session named '{session_name}' is running, but its event_file target could not be inspected: {attach_error}\n\
Grant ALTER ANY EVENT SESSION plus VIEW SERVER STATE (or VIEW SERVER PERFORMANCE STATE on SQL Server 2022+), or have a DBA create the server-scoped session with an event_file target."
                                    ));
                                }
                            }
                        }
                        CaptureStorageMode::InMemory => {
                            match server_session_has_ring_buffer(client, session_name).await {
                                Ok(true) => {
                                    return Ok(ActiveSession {
                                        session_name: session_name.to_string(),
                                        storage_mode: options.storage_mode,
                                        file_pattern: None,
                                        read_only: true,
                                    });
                                }
                                Ok(false) => {
                                    return Err(format!(
                                        "In-memory mode requires a server-scoped Extended Events session with a ring_buffer target.\n\
Failed to create/start server-scoped session: {server_error}\n\
An existing session named '{session_name}' is running, but it does not expose a ring_buffer target that this app can read.\n\
Grant ALTER ANY EVENT SESSION plus VIEW SERVER STATE (or VIEW SERVER PERFORMANCE STATE on SQL Server 2022+), or have a DBA create the server-scoped session with a ring_buffer target."
                                    ));
                                }
                                Err(attach_error) => {
                                    return Err(format!(
                                        "In-memory mode requires a server-scoped Extended Events session with a ring_buffer target.\n\
Failed to create/start server-scoped session: {server_error}\n\
An existing session named '{session_name}' is running, but its ring_buffer target could not be inspected: {attach_error}\n\
Grant ALTER ANY EVENT SESSION plus VIEW SERVER STATE (or VIEW SERVER PERFORMANCE STATE on SQL Server 2022+), or have a DBA create the server-scoped session with a ring_buffer target."
                                    ));
                                }
                            }
                        }
                    }
                }

                let message = match options.storage_mode {
                    CaptureStorageMode::Files => format!(
                        "Trace files mode requires a server-scoped Extended Events session with an event_file target.\n\
Failed to create/start server-scoped session: {server_error}\n\
Grant ALTER ANY EVENT SESSION plus VIEW SERVER STATE (or VIEW SERVER PERFORMANCE STATE on SQL Server 2022+), or have a DBA create the server-scoped session with an event_file target."
                    ),
                    CaptureStorageMode::InMemory => format!(
                        "In-memory mode requires a server-scoped Extended Events session with a ring_buffer target.\n\
Failed to create/start server-scoped session: {server_error}\n\
Grant ALTER ANY EVENT SESSION plus VIEW SERVER STATE (or VIEW SERVER PERFORMANCE STATE on SQL Server 2022+), or have a DBA create the server-scoped session with a ring_buffer target."
                    ),
                };

                Err(message)
            } else {
                Err(format!(
                    "Failed to create/start server-scoped Extended Events session: {server_error}"
                ))
            }
        }
    }
}

async fn start_server_scoped_session(
    client: &mut SqlClient,
    session_name: &str,
    storage_mode: CaptureStorageMode,
) -> Result<SessionTargetInfo, String> {
    use tiberius::Query;

    let create_sql = build_create_and_start_sql(storage_mode);
    let mut query = Query::new(&create_sql);
    query.bind(session_name);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;
    let results = stream.into_results().await.map_err(|e| format!("{e}"))?;

    for result_set in results {
        for row in result_set {
            let file_pattern = row
                .get::<&str, _>("file_pattern")
                .map(normalize_event_file_pattern)
                .or_else(|| {
                    row.get::<&str, _>("base_file")
                        .map(normalize_event_file_pattern)
                });

            return Ok(SessionTargetInfo { file_pattern });
        }
    }

    if matches!(storage_mode, CaptureStorageMode::InMemory) {
        Ok(SessionTargetInfo { file_pattern: None })
    } else {
        Err("Extended Events session started, but no event_file target path was returned.".into())
    }
}

async fn server_session_has_ring_buffer(
    client: &mut SqlClient,
    session_name: &str,
) -> Result<bool, String> {
    use tiberius::Query;

    let mut query = Query::new(
        "SELECT CAST(
            CASE WHEN EXISTS (
                SELECT 1
                FROM sys.dm_xe_sessions s
                INNER JOIN sys.dm_xe_session_targets st
                    ON st.event_session_address = s.address
                WHERE s.name = @P1
                  AND st.target_name = N'ring_buffer'
            ) THEN 1 ELSE 0 END
            AS int
        ) AS has_ring_buffer;",
    );
    query.bind(session_name);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;
    let rows = stream.into_results().await.map_err(|e| format!("{e}"))?;

    let has_ring_buffer = rows
        .first()
        .and_then(|set| set.first())
        .and_then(|row| row.get::<i32, _>("has_ring_buffer"))
        .unwrap_or(0)
        == 1;

    Ok(has_ring_buffer)
}
async fn server_session_exists(client: &mut SqlClient, session_name: &str) -> Result<bool, String> {
    use tiberius::Query;

    let mut query = Query::new(
        "SELECT CAST(CASE WHEN EXISTS (SELECT 1 FROM sys.dm_xe_sessions WHERE name = @P1) THEN 1 ELSE 0 END AS int) AS session_exists;",
    );
    query.bind(session_name);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;
    let rows = stream.into_results().await.map_err(|e| format!("{e}"))?;

    let exists = rows
        .first()
        .and_then(|set| set.first())
        .and_then(|row| row.get::<i32, _>("session_exists"))
        .unwrap_or(0)
        == 1;

    Ok(exists)
}

async fn server_session_get_event_file_pattern(
    client: &mut SqlClient,
    session_name: &str,
) -> Result<Option<String>, String> {
    use tiberius::Query;

    let mut query = Query::new(
        "SELECT TOP (1)
            CAST(f.value AS nvarchar(4000)) AS file_name
        FROM sys.server_event_sessions s
        INNER JOIN sys.server_event_session_targets t
            ON t.event_session_id = s.event_session_id
        INNER JOIN sys.server_event_session_fields f
            ON f.event_session_id = s.event_session_id
           AND f.object_id = t.target_id
        WHERE s.name = @P1
          AND (t.name = N'event_file' OR t.name = N'package0.event_file')
          AND f.name = N'filename';",
    );
    query.bind(session_name);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;
    let rows = stream.into_results().await.map_err(|e| format!("{e}"))?;

    let file_name = rows
        .first()
        .and_then(|set| set.first())
        .and_then(|row| row.get::<&str, _>("file_name"))
        .map(normalize_event_file_pattern);

    Ok(file_name)
}

fn split_event_file_pattern(file_pattern: &str) -> Option<(&str, &str)> {
    let last_separator = file_pattern.rfind(['\\', '/'])?;
    let directory = &file_pattern[..=last_separator];
    let pattern = &file_pattern[last_separator + 1..];

    if directory.is_empty() || pattern.is_empty() {
        return None;
    }

    Some((directory, pattern))
}

async fn enumerate_event_files(
    client: &mut SqlClient,
    file_pattern: &str,
) -> Result<Vec<String>, String> {
    use tiberius::Query;

    let (directory, pattern) = split_event_file_pattern(file_pattern).ok_or_else(|| {
        format!("Invalid trace file pattern returned by SQL Server: {file_pattern}")
    })?;

    let mut query = Query::new(
        "SELECT CAST(full_filesystem_path AS nvarchar(4000)) AS full_path
        FROM sys.dm_os_enumerate_filesystem(@P1, @P2)
        WHERE is_directory = 0;",
    );
    query.bind(directory);
    query.bind(pattern);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;
    let rows = stream.into_results().await.map_err(|e| format!("{e}"))?;

    let files = rows
        .first()
        .map(|set| {
            set.iter()
                .filter_map(|row| row.get::<&str, _>("full_path").map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(files)
}

async fn try_cleanup_trace_files(
    client: &mut SqlClient,
    file_pattern: &str,
) -> Result<usize, String> {
    use tiberius::Query;

    let files = enumerate_event_files(client, file_pattern).await?;
    if files.is_empty() {
        return Ok(0);
    }

    let deleted_count = files.len();
    let mut failures = Vec::new();
    for path in files {
        let mut query = Query::new("EXEC sys.xp_delete_files @P1;");
        query.bind(path.as_str());

        match query.query(client).await {
            Ok(stream) => {
                if let Err(error) = stream.into_results().await {
                    failures.push(format!("{path}: {error}"));
                }
            }
            Err(error) => {
                failures.push(format!("{path}: {error}"));
            }
        }
    }

    if failures.is_empty() {
        Ok(deleted_count)
    } else {
        Err(format!(
            "Failed to delete {} trace file(s): {}",
            failures.len(),
            failures.join(" | ")
        ))
    }
}

fn is_permission_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("permission")
        || lower.contains("not authorized")
        || lower.contains("access is denied")
        || lower.contains("create any event session")
        || lower.contains("view server state")
        || lower.contains("view server performance state")
        || lower.contains("alter any event session")
}

fn normalize_event_file_pattern(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return "*.xel".to_string();
    }

    let lower = trimmed.to_ascii_lowercase();
    if trimmed.contains('*') {
        trimmed.to_string()
    } else if lower.ends_with(".xel") {
        format!("{}*.xel", &trimmed[..trimmed.len() - 4])
    } else {
        format!("{trimmed}*.xel")
    }
}

fn derive_event_status(event_name: &str) -> &'static str {
    if event_name.ends_with("_starting") || event_name == "module_start" {
        "starting"
    } else {
        "completed"
    }
}

fn should_ignore_program_name(program_name: &str) -> bool {
    let normalized = program_name.trim().to_ascii_lowercase();
    normalized.starts_with("simplesqlprofiler") || normalized.starts_with("simplesqlquerywindow")
}

fn should_ignore_sql_text(sql_text: &str, current_statement: &str) -> bool {
    let statement = if current_statement.trim().is_empty() {
        sql_text
    } else {
        current_statement
    };

    let normalized = statement.trim().trim_end_matches(';').to_ascii_lowercase();

    normalized == "sp_reset_connection" || normalized == "exec sp_reset_connection"
}

fn infer_database_name_from_sql(sql_text: &str, current_statement: &str) -> Option<String> {
    fn parse_use_statement(input: &str) -> Option<String> {
        let trimmed = input.trim_start_matches('\u{feff}').trim_start();
        let lower = trimmed.to_ascii_lowercase();
        if !lower.starts_with("use ") {
            return None;
        }

        let remainder = trimmed[4..].trim_start();
        if remainder.is_empty() {
            return None;
        }

        if let Some(stripped) = remainder.strip_prefix('[') {
            let end = stripped.find(']')?;
            let name = stripped[..end].trim();
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        } else {
            let end = remainder
                .find(|c: char| c == ';' || c.is_whitespace())
                .unwrap_or(remainder.len());
            let name = remainder[..end].trim().trim_matches('"');
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
    }

    parse_use_statement(current_statement).or_else(|| parse_use_statement(sql_text))
}

fn row_string(row: &tiberius::Row, column: &str) -> String {
    row.try_get::<&str, _>(column)
        .ok()
        .flatten()
        .unwrap_or("")
        .to_string()
}

fn row_i64(row: &tiberius::Row, column: &str) -> i64 {
    row.try_get::<i64, _>(column).ok().flatten().unwrap_or(0)
}

fn row_i32(row: &tiberius::Row, column: &str) -> i32 {
    row.try_get::<i32, _>(column).ok().flatten().unwrap_or(0)
}

async fn stop_and_close_session(
    client: &mut SqlClient,
    session: &ActiveSession,
) -> Result<StopSessionFeedback, String> {
    use tiberius::Query;

    if session.read_only {
        return Ok(StopSessionFeedback::default());
    }

    let stop_sql = build_stop_and_drop_sql();
    let mut query = Query::new(&stop_sql);
    query.bind(&session.session_name);

    query
        .query(client)
        .await
        .map_err(|e| format!("Failed to stop/drop Extended Events session: {e}"))?
        .into_results()
        .await
        .map_err(|e| format!("Failed to confirm Extended Events session stop/drop: {e}"))?;

    let mut feedback = StopSessionFeedback::default();

    if matches!(session.storage_mode, CaptureStorageMode::Files) {
        if let Some(file_pattern) = session.file_pattern.as_deref() {
            match try_cleanup_trace_files(client, file_pattern).await {
                Ok(deleted_count) if deleted_count > 0 => {
                    feedback.toast = Some(if deleted_count == 1 {
                        "Deleted 1 trace file from the server.".to_string()
                    } else {
                        format!("Deleted {deleted_count} trace files from the server.")
                    });
                }
                Ok(_) => {}
                Err(error) => {
                    feedback.note = Some(format!(
                        "Capture stopped, but trace file cleanup failed: {error}"
                    ));
                }
            }
        }
    }

    Ok(feedback)
}

async fn stop_session(client: &mut SqlClient, session: &ActiveSession) -> Result<(), String> {
    use tiberius::Query;

    if session.read_only {
        return Ok(());
    }

    let stop_sql = build_stop_sql();
    let mut query = Query::new(&stop_sql);
    query.bind(&session.session_name);

    query
        .query(client)
        .await
        .map_err(|e| format!("Failed to stop Extended Events session: {e}"))?
        .into_results()
        .await
        .map_err(|e| format!("Failed to confirm Extended Events session stop: {e}"))?;

    Ok(())
}

async fn stop_polling_gracefully(
    poll_run_flag: &mut Option<Arc<AtomicBool>>,
    polling_task: &mut Option<tauri::async_runtime::JoinHandle<()>>,
) {
    if let Some(flag) = poll_run_flag.take() {
        flag.store(false, Ordering::Release);
    }

    if let Some(task) = polling_task.take() {
        let mut task = task;
        if tokio::time::timeout(std::time::Duration::from_millis(1500), &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}

async fn poll_session_events(
    client: &mut SqlClient,
    session: &ActiveSession,
    bookmark: Option<&SessionReadBookmark>,
    last_timestamp: &str,
    last_event_sequence: i64,
) -> Result<Vec<PolledEvent>, String> {
    use tiberius::Query;

    let query = match session.storage_mode {
        CaptureStorageMode::Files => {
            let mut query = Query::new(XE_POLL_EVENT_FILE);
            query.bind(
                session
                    .file_pattern
                    .as_deref()
                    .ok_or_else(|| "Event file mode is missing the file pattern.".to_string())?,
            );
            query.bind(bookmark.map(|bookmark| bookmark.file_name.as_str()));
            query.bind(bookmark.map(|bookmark| bookmark.file_offset));
            query
        }
        CaptureStorageMode::InMemory => {
            let mut query = Query::new(XE_POLL_EVENTS);
            query.bind(&session.session_name);
            query.bind(last_timestamp);
            query.bind(last_event_sequence);
            query
        }
    };

    let stream = query
        .query(client)
        .await
        .map_err(|e| format!("Extended Events poll query failed: {e}"))?;

    let rows = stream
        .into_results()
        .await
        .map_err(|e| format!("Failed to read Extended Events poll results: {e}"))?;

    let mut events = Vec::new();

    if let Some(result_set) = rows.first() {
        for row in result_set {
            let event_name = row_string(row, "event_name");
            let file_name = row_string(row, "file_name");
            let file_offset = row_i64(row, "file_offset");
            let start_time = row_string(row, "start_time");
            let cursor_time = row_string(row, "cursor_time");
            let event_sequence = row_i64(row, "event_sequence");
            let duration_us = row_i64(row, "duration_us");
            let cpu_time_us = row_i64(row, "cpu_time_us");
            let elapsed_time = (duration_us / 1000) as i32;
            let cpu_time = (cpu_time_us / 1000) as i32;

            let logical_reads = row_i64(row, "logical_reads");
            let physical_reads = row_i64(row, "physical_reads");
            let writes = row_i64(row, "writes");
            let row_count = row_i64(row, "row_count");

            let sql_text = row_string(row, "sql_text");
            let current_statement_raw = row_string(row, "current_statement");
            let database_name_raw = row_string(row, "database_name");
            let database_id_value = row_i32(row, "database_id_value");
            let login_name = row_string(row, "login_name");
            let host_name = row_string(row, "host_name");
            let program_name = row_string(row, "program_name");
            if should_ignore_program_name(&program_name) {
                continue;
            }
            let session_id = row_i32(row, "session_id");
            let database_name = if database_name_raw.trim().is_empty() && database_id_value > 0 {
                format!("dbid:{database_id_value}")
            } else {
                database_name_raw
            };

            let current_statement = if matches!(
                event_name.as_str(),
                "rpc_starting"
                    | "rpc_completed"
                    | "sp_statement_starting"
                    | "sp_statement_completed"
                    | "sql_statement_starting"
                    | "sql_statement_completed"
                    | "exec_prepared_sql"
                    | "prepare_sql"
                    | "module_start"
                    | "module_end"
            ) {
                current_statement_raw
            } else {
                String::new()
            };
            let sql_text = if sql_text.is_empty() {
                current_statement.clone()
            } else {
                sql_text
            };
            let database_name = infer_database_name_from_sql(&sql_text, &current_statement)
                .unwrap_or(database_name);
            if should_ignore_sql_text(&sql_text, &current_statement) {
                continue;
            }

            events.push(PolledEvent {
                event: QueryEvent {
                    id: String::new(),
                    session_id,
                    start_time,
                    event_name,
                    database_name,
                    cpu_time,
                    elapsed_time,
                    physical_reads,
                    writes,
                    logical_reads,
                    row_count,
                    sql_text,
                    current_statement,
                    login_name,
                    host_name,
                    program_name,
                    captured_at: String::new(),
                    event_status: String::new(),
                },
                cursor_time,
                event_sequence,
                file_name,
                file_offset,
            });
        }
    }

    Ok(events)
}

fn is_transient_session_poll_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    (lower.contains("event session") && lower.contains("does not exist"))
        || (lower.contains("target")
            && lower.contains("ring_buffer")
            && lower.contains("does not exist"))
        || lower.contains("no files matched")
        || lower.contains("cannot find the file specified")
        || lower.contains("cannot find the path specified")
        || lower.contains("the system cannot find the file specified")
        || lower.contains("the system cannot find the path specified")
}

async fn execute_user_query(
    config: &ConnectionConfig,
    sql: &str,
) -> Result<QueryResultData, String> {
    let mut client = db::connect_for_query_window(config).await?;
    let stream = client.simple_query(sql).await.map_err(|e| format!("{e}"))?;

    let result_sets = stream.into_results().await.map_err(|e| format!("{e}"))?;

    // Use the first result set that has columns
    for result_set in &result_sets {
        if result_set.is_empty() {
            continue;
        }

        let columns: Vec<String> = result_set[0]
            .columns()
            .iter()
            .map(|c| c.name().to_string())
            .collect();

        if columns.is_empty() {
            continue;
        }

        let rows: Vec<Vec<serde_json::Value>> = result_set
            .iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(i, _)| row_value_to_json(row, i))
                    .collect()
            })
            .collect();

        return Ok(QueryResultData { columns, rows });
    }

    Ok(QueryResultData {
        columns: vec![],
        rows: vec![],
    })
}

fn row_value_to_json(row: &tiberius::Row, idx: usize) -> serde_json::Value {
    use serde_json::Value;

    // Try common types in order of likelihood
    if let Some(v) = row.try_get::<&str, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<i32, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<i64, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<i16, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<u8, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<f64, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<f32, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row.try_get::<bool, _>(idx).ok().flatten() {
        return Value::from(v);
    }
    if let Some(v) = row
        .try_get::<tiberius::numeric::Numeric, _>(idx)
        .ok()
        .flatten()
    {
        return Value::from(v.to_string());
    }
    if let Some(v) = row.try_get::<&[u8], _>(idx).ok().flatten() {
        let hex: String = v.iter().map(|b| format!("{b:02X}")).collect();
        return Value::from(format!("0x{hex}"));
    }

    // For datetime and other types, use Debug formatting as fallback
    Value::from(format!("{:?}", row.try_get::<&str, _>(idx)))
}
