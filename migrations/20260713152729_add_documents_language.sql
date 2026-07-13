alter table documents
    add column language text check (language is null or language in ('en', 'cyr'));
