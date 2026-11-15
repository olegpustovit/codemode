UPDATE tool_policies SET action='allow' WHERE action IN ('approve','ask');
ALTER TABLE tool_policies DROP CONSTRAINT tool_policies_action_check;
ALTER TABLE tool_policies ADD CONSTRAINT tool_policies_action_check CHECK(action IN ('allow','block'));
-- Finish historical pauses without repeating any upstream side effects.
UPDATE executions SET status='failed',error='Approval workflow removed; execution was not replayed',finished_at=now() WHERE status='paused';
UPDATE tool_calls SET outcome='cancelled',error='Approval workflow removed; call was not dispatched' WHERE outcome='awaiting_approval';
ALTER TABLE executions DROP CONSTRAINT executions_status_check;
ALTER TABLE executions ADD CONSTRAINT executions_status_check CHECK(status IN ('running','completed','failed','timed_out','expired'));
DROP TABLE approval_calls, paused_runs;
