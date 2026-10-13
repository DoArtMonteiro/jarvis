-- Durable jobs: a job counts how often it has been claimed, and every task
-- created for a job is recorded at its position in the attempt that created
-- it. The ledger is what lets a later attempt tell which steps already
-- completed.
ALTER TABLE jobs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;

CREATE TRIGGER jobs_attempts_monotonic
BEFORE UPDATE OF attempts ON jobs
WHEN NEW.attempts < OLD.attempts
BEGIN
    SELECT RAISE(ABORT, 'job attempts never decrease');
END;

-- One row per task created for a job: the attempt that created it and its
-- position (`seq`, counted from 1) within that attempt. Insert-only.
CREATE TABLE job_steps (
    task_id TEXT PRIMARY KEY NOT NULL REFERENCES tasks (task_id),
    job_id  TEXT NOT NULL REFERENCES jobs (job_id),
    attempt INTEGER NOT NULL CHECK (attempt >= 1),
    seq     INTEGER NOT NULL CHECK (seq >= 1),
    UNIQUE (job_id, attempt, seq)
) STRICT;

CREATE INDEX job_steps_position ON job_steps (job_id, seq);

CREATE TRIGGER job_steps_belong_to_the_task_job
BEFORE INSERT ON job_steps
WHEN (SELECT job_id FROM tasks WHERE task_id = NEW.task_id) IS NOT NEW.job_id
BEGIN
    SELECT RAISE(ABORT, 'a step must belong to the job of its task');
END;

CREATE TRIGGER job_steps_no_update
BEFORE UPDATE ON job_steps
BEGIN
    SELECT RAISE(ABORT, 'job_steps is insert-only');
END;

CREATE TRIGGER job_steps_no_delete
BEFORE DELETE ON job_steps
BEGIN
    SELECT RAISE(ABORT, 'job_steps is insert-only');
END;
