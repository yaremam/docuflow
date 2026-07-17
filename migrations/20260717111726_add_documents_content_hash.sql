alter table documents
  add column content_hash text;

create index documents_tenant_id_content_hash_idx on documents (tenant_id, content_hash);
