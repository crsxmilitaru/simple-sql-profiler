use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use crate::db::{self, ConnectionConfig, SqlClient};

const MIN_TIMESTAMP: &str = "1900-01-01T00:00:00.000";
const XE_POLL_PAGE_SIZE: usize = 5000;
const XE_POLL_MAX_DRAIN_PASSES: usize = 8;

const XE_SESSION_NAME: &str = "SimpleSQLProfilerXE";

const XE_CREATE_AND_START: &str = "
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

SET @sql = N'
CREATE EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER
ADD EVENT sqlserver.login(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.logout(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.rpc_starting(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.rpc_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sql_batch_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sql_batch_starting(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sp_statement_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sql_statement_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.exec_prepared_sql(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.prepare_sql(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.unprepare_sql(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
)
ADD TARGET package0.ring_buffer(
    SET max_memory = 1024,
        max_events_limit = 5000
)
WITH (
    MAX_MEMORY = 8 MB,
    EVENT_RETENTION_MODE = ALLOW_SINGLE_EVENT_LOSS,
    MAX_DISPATCH_LATENCY = 1 SECONDS,
    TRACK_CAUSALITY = OFF,
    STARTUP_STATE = OFF
);';
EXEC(@sql);

SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON SERVER STATE = START;';
EXEC(@sql);

SELECT @session_name AS session_name;
";

const XE_STOP_AND_DROP: &str = "
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
";

const XE_CREATE_AND_START_DB: &str = "
DECLARE @session_name sysname = @P1;
DECLARE @sql nvarchar(max);

IF EXISTS (SELECT 1 FROM sys.database_event_sessions WHERE name = @session_name)
BEGIN
    BEGIN TRY
        SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON DATABASE STATE = STOP;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;

    BEGIN TRY
        SET @sql = N'DROP EVENT SESSION ' + QUOTENAME(@session_name) + N' ON DATABASE;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;
END

SET @sql = N'
CREATE EVENT SESSION ' + QUOTENAME(@session_name) + N' ON DATABASE
ADD EVENT sqlserver.rpc_starting(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.rpc_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sql_batch_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sql_batch_starting(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sp_statement_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.sql_statement_completed(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.exec_prepared_sql(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.prepare_sql(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
),
ADD EVENT sqlserver.unprepare_sql(
    ACTION(
        package0.event_sequence,
        sqlserver.session_id,
        sqlserver.client_app_name,
        sqlserver.client_hostname,
        sqlserver.server_principal_name,
        sqlserver.database_name,
        sqlserver.database_id,
        sqlserver.sql_text
    )
    WHERE ([sqlserver].[client_app_name] NOT LIKE N''%SimpleSQLProfiler%'')
)
ADD TARGET package0.ring_buffer(
    SET max_memory = 1024,
        max_events_limit = 5000
)
WITH (
    MAX_MEMORY = 8 MB,
    EVENT_RETENTION_MODE = ALLOW_SINGLE_EVENT_LOSS,
    MAX_DISPATCH_LATENCY = 1 SECONDS,
    TRACK_CAUSALITY = OFF,
    STARTUP_STATE = OFF
);';
EXEC(@sql);

SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON DATABASE STATE = START;';
EXEC(@sql);

SELECT @session_name AS session_name;
";

const XE_STOP_AND_DROP_DB: &str = "
DECLARE @session_name sysname = @P1;
DECLARE @sql nvarchar(max);

IF EXISTS (SELECT 1 FROM sys.database_event_sessions WHERE name = @session_name)
BEGIN
    BEGIN TRY
        SET @sql = N'ALTER EVENT SESSION ' + QUOTENAME(@session_name) + N' ON DATABASE STATE = STOP;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;

    BEGIN TRY
        SET @sql = N'DROP EVENT SESSION ' + QUOTENAME(@session_name) + N' ON DATABASE;';
        EXEC(@sql);
    END TRY
    BEGIN CATCH
    END CATCH;
END
";

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
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'sp_statement_completed', N'sql_statement_completed')
             AND LEN(statement_text) > 0 THEN statement_text
        WHEN event_name = N'prepare_sql' AND LEN(prepared_statement_text) > 0 THEN prepared_statement_text
        WHEN event_name = N'exec_prepared_sql' AND LEN(sql_text_action) > 0 THEN sql_text_action
        WHEN LEN(batch_text) > 0 THEN batch_text
        WHEN LEN(options_text) > 0 THEN options_text
        ELSE sql_text_action
    END AS sql_text,
    CASE
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'sp_statement_completed', N'sql_statement_completed')
            THEN statement_text
        WHEN event_name = N'prepare_sql' THEN prepared_statement_text
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
    N'login',
    N'logout',
    N'rpc_starting',
    N'rpc_completed',
    N'sql_batch_starting',
    N'sql_batch_completed',
    N'sp_statement_completed',
    N'sql_statement_completed',
    N'exec_prepared_sql',
    N'prepare_sql',
    N'unprepare_sql'
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

const XE_POLL_EVENTS_DB: &str = "
WITH xe_data AS (
    SELECT CAST(st.target_data AS xml) AS target_data
    FROM sys.dm_xe_database_sessions s
    INNER JOIN sys.dm_xe_database_session_targets st
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
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'sp_statement_completed', N'sql_statement_completed')
             AND LEN(statement_text) > 0 THEN statement_text
        WHEN event_name = N'prepare_sql' AND LEN(prepared_statement_text) > 0 THEN prepared_statement_text
        WHEN event_name = N'exec_prepared_sql' AND LEN(sql_text_action) > 0 THEN sql_text_action
        WHEN LEN(batch_text) > 0 THEN batch_text
        WHEN LEN(options_text) > 0 THEN options_text
        ELSE sql_text_action
    END AS sql_text,
    CASE
        WHEN event_name IN (N'rpc_starting', N'rpc_completed', N'sp_statement_completed', N'sql_statement_completed')
            THEN statement_text
        WHEN event_name = N'prepare_sql' THEN prepared_statement_text
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
    N'login',
    N'logout',
    N'rpc_starting',
    N'rpc_completed',
    N'sql_batch_starting',
    N'sql_batch_completed',
    N'sp_statement_completed',
    N'sql_statement_completed',
    N'exec_prepared_sql',
    N'prepare_sql',
    N'unprepare_sql'
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
}

#[derive(Debug, Clone)]
struct PolledEvent {
    event: QueryEvent,
    event_sequence: i64,
    cursor_time: String,
}

#[derive(Debug, Clone, Copy)]
enum SessionScope {
    Server,
    Database,
}

#[derive(Debug, Clone)]
struct ActiveSession {
    session_name: String,
    scope: SessionScope,
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
    ) {
        let _ = app.emit(
            "profiler-status",
            ProfilerStatus {
                connected,
                capturing,
                error,
            },
        );
    }

    fn abort_polling_task(polling_task: &mut Option<tauri::async_runtime::JoinHandle<()>>) {
        if let Some(task) = polling_task.take() {
            task.abort();
        }
    }

    fn stop_polling_now(
        poll_run_flag: &mut Option<Arc<AtomicBool>>,
        polling_task: &mut Option<tauri::async_runtime::JoinHandle<()>>,
    ) {
        if let Some(flag) = poll_run_flag.take() {
            flag.store(false, Ordering::Release);
        }
        abort_polling_task(polling_task);
    }

    while let Some(cmd) = rx.recv().await {
        match cmd {
            ProfilerCommand::Connect { config, reply } => {
                stop_polling_now(&mut poll_run_flag, &mut polling_task);
                if let (Some(c), Some(session)) = (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_and_close_session(c, session).await;
                }
                active_session = None;

                match db::connect(&config).await {
                    Ok(c) => {
                        control_client = Some(c);
                        active_config = Some(config);
                        emit_status(&app, true, false, None);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        control_client = None;
                        active_config = None;
                        emit_status(&app, false, false, Some(e.clone()));
                        let _ = reply.send(Err(e));
                    }
                }
            }
            ProfilerCommand::Disconnect { reply } => {
                stop_polling_now(&mut poll_run_flag, &mut polling_task);

                let stop_error = if let (Some(c), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    stop_and_close_session(c, session).await.err()
                } else {
                    None
                };
                control_client = None;
                active_config = None;
                active_session = None;
                emit_status(&app, false, false, stop_error);
                let _ = reply.send(Ok(()));
            }
            ProfilerCommand::StartCapture { reply } => {
                if control_client.is_none() {
                    let _ = reply.send(Err("Not connected".into()));
                    continue;
                }

                stop_polling_now(&mut poll_run_flag, &mut polling_task);
                if let (Some(control), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    let _ = stop_and_close_session(control, session).await;
                    active_session = None;
                }

                let session = match control_client.as_mut() {
                    Some(control) => match start_session(control, XE_SESSION_NAME).await {
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
                            Some(
                                "Attached to an existing server XE session in read-only mode. Stop will only stop local polling.".to_string(),
                            )
                        } else if matches!(session.scope, SessionScope::Database) {
                            Some(
                                "Using database-scoped XE fallback. This captures events for the connected database context, not all databases on the server.".to_string(),
                            )
                        } else {
                            None
                        };
                        emit_status(&app, true, true, capture_note);
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
                        emit_status(&app, true, false, Some(message.clone()));
                        let _ = reply.send(Err(message));
                    }
                }
            }
            ProfilerCommand::StopCapture { reply } => {
                stop_polling_now(&mut poll_run_flag, &mut polling_task);
                let stop_result = if let (Some(c), Some(session)) =
                    (control_client.as_mut(), active_session.as_ref())
                {
                    stop_and_close_session(c, session).await
                } else {
                    Ok(())
                };
                active_session = None;

                match stop_result {
                    Ok(()) => {
                        emit_status(&app, control_client.is_some(), false, None);
                        let _ = reply.send(Ok(()));
                    }
                    Err(e) => {
                        emit_status(&app, control_client.is_some(), false, Some(e.clone()));
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

    stop_polling_now(&mut poll_run_flag, &mut polling_task);
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

        let mut last_timestamp = String::from(MIN_TIMESTAMP);
        let mut last_event_sequence = -1_i64;
        let mut seen_without_sequence_at_timestamp = HashSet::<String>::new();
        let mut transient_failures = 0_u32;
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(300));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if !run_flag.load(Ordering::Acquire) {
                break;
            }
            interval.tick().await;
            if !run_flag.load(Ordering::Acquire) {
                break;
            }

            let mut pass = 0usize;
            loop {
                let events = match poll_session_events(
                    &mut poll_client,
                    &session,
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
                for mut polled in events {
                    if !run_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let ts = polled.cursor_time;
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
                            // Prevent replaying sequence=0 rows for same timestamp forever.
                            last_event_sequence = 0;
                        }
                    }

                    polled.event.id = uuid::Uuid::new_v4().to_string();
                    polled.event.captured_at = now.clone();
                    polled.event.event_status = "completed".into();
                    let _ = app.emit("query-event", &polled.event);
                }

                pass += 1;
                if event_count < XE_POLL_PAGE_SIZE || pass >= XE_POLL_MAX_DRAIN_PASSES {
                    break;
                }
            }
        }
    })
}

async fn start_session(
    client: &mut SqlClient,
    session_name: &str,
) -> Result<ActiveSession, String> {
    match start_server_scoped_session(client, session_name).await {
        Ok(()) => Ok(ActiveSession {
            session_name: session_name.to_string(),
            scope: SessionScope::Server,
            read_only: false,
        }),
        Err(server_error) => {
            if is_permission_error(&server_error) {
                if matches!(server_session_exists(client, session_name).await, Ok(true)) {
                    if matches!(
                        server_session_has_ring_buffer(client, session_name).await,
                        Ok(true)
                    ) {
                        return Ok(ActiveSession {
                            session_name: session_name.to_string(),
                            scope: SessionScope::Server,
                            read_only: true,
                        });
                    }
                }

                match start_database_scoped_session(client, session_name).await {
                    Ok(()) => Ok(ActiveSession {
                        session_name: session_name.to_string(),
                        scope: SessionScope::Database,
                        read_only: false,
                    }),
                    Err(database_error) => Err(format!(
                        "Failed to start Extended Events capture.\n\
Server-scoped session error: {server_error}\n\
Database-scoped fallback error: {database_error}"
                    )),
                }
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
) -> Result<(), String> {
    use tiberius::Query;

    let mut query = Query::new(XE_CREATE_AND_START);
    query.bind(session_name);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;

    stream.into_results().await.map_err(|e| format!("{e}"))?;

    Ok(())
}

async fn start_database_scoped_session(
    client: &mut SqlClient,
    session_name: &str,
) -> Result<(), String> {
    use tiberius::Query;

    let mut query = Query::new(XE_CREATE_AND_START_DB);
    query.bind(session_name);

    let stream = query.query(client).await.map_err(|e| format!("{e}"))?;

    stream.into_results().await.map_err(|e| format!("{e}"))?;

    Ok(())
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

fn is_permission_error(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("permission")
        || lower.contains("not authorized")
        || lower.contains("access is denied")
        || lower.contains("view server state")
        || lower.contains("view server performance state")
        || lower.contains("alter any event session")
}

async fn stop_and_close_session(
    client: &mut SqlClient,
    session: &ActiveSession,
) -> Result<(), String> {
    use tiberius::Query;

    if session.read_only {
        return Ok(());
    }

    let stop_sql = match session.scope {
        SessionScope::Server => XE_STOP_AND_DROP,
        SessionScope::Database => XE_STOP_AND_DROP_DB,
    };

    let mut query = Query::new(stop_sql);
    query.bind(&session.session_name);

    query
        .query(client)
        .await
        .map_err(|e| format!("Failed to stop/drop Extended Events session: {e}"))?
        .into_results()
        .await
        .map_err(|e| format!("Failed to confirm Extended Events session stop/drop: {e}"))?;

    Ok(())
}

async fn poll_session_events(
    client: &mut SqlClient,
    session: &ActiveSession,
    last_timestamp: &str,
    last_event_sequence: i64,
) -> Result<Vec<PolledEvent>, String> {
    use tiberius::Query;

    let poll_sql = match session.scope {
        SessionScope::Server => XE_POLL_EVENTS,
        SessionScope::Database => XE_POLL_EVENTS_DB,
    };

    let mut query = Query::new(poll_sql);
    query.bind(&session.session_name);
    query.bind(last_timestamp);
    query.bind(last_event_sequence);

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
            let event_name: String = row.get::<&str, _>("event_name").unwrap_or("").to_string();
            if !matches!(
                event_name.as_str(),
                "login"
                    | "logout"
                    | "rpc_starting"
                    | "rpc_completed"
                    | "sql_batch_starting"
                    | "sql_batch_completed"
                    | "sp_statement_completed"
                    | "sql_statement_completed"
                    | "exec_prepared_sql"
                    | "prepare_sql"
                    | "unprepare_sql"
            ) {
                continue;
            }
            let start_time: String = row.get::<&str, _>("start_time").unwrap_or("").to_string();
            let cursor_time: String = row.get::<&str, _>("cursor_time").unwrap_or("").to_string();
            let event_sequence: i64 = row.get::<i64, _>("event_sequence").unwrap_or(0);

            let duration_us: i64 = row.get::<i64, _>("duration_us").unwrap_or(0);
            let cpu_time_us: i64 = row.get::<i64, _>("cpu_time_us").unwrap_or(0);
            let elapsed_time = (duration_us / 1000) as i32;
            let cpu_time = (cpu_time_us / 1000) as i32;

            let logical_reads: i64 = row.get::<i64, _>("logical_reads").unwrap_or(0);
            let physical_reads: i64 = row.get::<i64, _>("physical_reads").unwrap_or(0);
            let writes: i64 = row.get::<i64, _>("writes").unwrap_or(0);
            let row_count: i64 = row.get::<i64, _>("row_count").unwrap_or(0);

            let sql_text: String = row.get::<&str, _>("sql_text").unwrap_or("").to_string();
            let current_statement_raw: String = row
                .get::<&str, _>("current_statement")
                .unwrap_or("")
                .to_string();
            let database_name_raw: String = row
                .get::<&str, _>("database_name")
                .unwrap_or("")
                .to_string();
            let database_id_value: i32 = row.get::<i32, _>("database_id_value").unwrap_or(0);
            let login_name: String = row.get::<&str, _>("login_name").unwrap_or("").to_string();
            let host_name: String = row.get::<&str, _>("host_name").unwrap_or("").to_string();
            let program_name: String = row.get::<&str, _>("program_name").unwrap_or("").to_string();
            let session_id: i32 = row.get::<i32, _>("session_id").unwrap_or(0);
            let database_name = if database_name_raw.trim().is_empty() && database_id_value > 0 {
                format!("dbid:{database_id_value}")
            } else {
                database_name_raw
            };

            let current_statement = if matches!(
                event_name.as_str(),
                "rpc_starting"
                    | "rpc_completed"
                    | "sp_statement_completed"
                    | "sql_statement_completed"
                    | "exec_prepared_sql"
                    | "prepare_sql"
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
                event_sequence,
                cursor_time,
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
