-- NULL represents grants issued before workspace URLs and accepts only the legacy resource.
ALTER TABLE oauth_grants ADD COLUMN resource_url text CHECK (length(resource_url) <= 2048);
