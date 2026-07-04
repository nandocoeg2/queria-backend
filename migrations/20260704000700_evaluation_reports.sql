create table if not exists evaluation_report (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  project_id uuid not null references project(id) on delete cascade,
  project_slug text not null,
  golden_question_file text not null,
  status text not null,
  total_questions integer not null,
  passed_questions integer not null,
  failed_questions integer not null,
  regression_score real not null,
  report_json jsonb not null,
  created_by uuid not null references user_account(id) on delete restrict,
  created_at timestamptz not null default now(),
  check (status in ('passed', 'failed')),
  check (total_questions >= 0),
  check (passed_questions >= 0),
  check (failed_questions >= 0),
  check (regression_score >= 0 and regression_score <= 1)
);

create index if not exists idx_evaluation_report_project_created
  on evaluation_report (project_id, created_at desc);

create index if not exists idx_evaluation_report_org_created
  on evaluation_report (organization_id, created_at desc);
