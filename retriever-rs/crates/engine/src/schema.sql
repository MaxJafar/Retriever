PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS revision(id INTEGER PRIMARY KEY CHECK(id=1),value INTEGER NOT NULL);
INSERT OR IGNORE INTO revision VALUES(1,0);
CREATE TABLE IF NOT EXISTS files(id INTEGER PRIMARY KEY, device TEXT NOT NULL,path TEXT NOT NULL,name TEXT NOT NULL,type TEXT NOT NULL,extension TEXT NOT NULL,text TEXT NOT NULL,detail TEXT NOT NULL,size INTEGER NOT NULL,mtime_ns INTEGER NOT NULL,digest TEXT NOT NULL,indexed_at REAL NOT NULL,scan_id TEXT NOT NULL DEFAULT '',UNIQUE(device,path));
CREATE TABLE IF NOT EXISTS devices(id TEXT PRIMARY KEY,status TEXT,checked_at REAL,report TEXT);
CREATE INDEX IF NOT EXISTS file_name_lookup ON files(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS file_path_lookup ON files(path COLLATE NOCASE);
CREATE VIRTUAL TABLE IF NOT EXISTS fts USING fts5(name,text,content='files',content_rowid='id',tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS file_insert AFTER INSERT ON files BEGIN
 INSERT INTO fts(rowid,name,text) VALUES(new.id,new.name,new.text); UPDATE revision SET value=value+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS file_delete AFTER DELETE ON files BEGIN
 INSERT INTO fts(fts,rowid,name,text) VALUES('delete',old.id,old.name,old.text); UPDATE revision SET value=value+1 WHERE id=1; END;
DROP TRIGGER IF EXISTS file_update;
CREATE TRIGGER file_update AFTER UPDATE ON files WHEN new.digest<>old.digest OR new.text<>old.text OR new.path<>old.path OR new.name<>old.name OR new.detail<>old.detail BEGIN
 INSERT INTO fts(fts,rowid,name,text) VALUES('delete',old.id,old.name,old.text);
 INSERT INTO fts(rowid,name,text) VALUES(new.id,new.name,new.text);
 DELETE FROM file_analysis WHERE file_id=new.id; DELETE FROM passages WHERE file_id=new.id;
 UPDATE revision SET value=value+1 WHERE id=1; END;
CREATE TABLE IF NOT EXISTS passages(id INTEGER PRIMARY KEY,file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,ordinal INTEGER NOT NULL,digest TEXT NOT NULL,text TEXT NOT NULL,coordinates TEXT NOT NULL,extraction_revision TEXT NOT NULL,UNIQUE(file_id,ordinal));
CREATE TABLE IF NOT EXISTS vectors(passage_id INTEGER NOT NULL REFERENCES passages(id) ON DELETE CASCADE,fingerprint TEXT NOT NULL,vector BLOB NOT NULL,dimensions INTEGER NOT NULL,PRIMARY KEY(passage_id,fingerprint));
CREATE TABLE IF NOT EXISTS snapshots(id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,dimensions INTEGER NOT NULL,rows TEXT NOT NULL,sha256 TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 0);
CREATE UNIQUE INDEX IF NOT EXISTS active_snapshot ON snapshots(fingerprint) WHERE active=1;
CREATE TABLE IF NOT EXISTS provider_probes(fingerprint TEXT PRIMARY KEY,role TEXT NOT NULL,result TEXT NOT NULL,checked REAL NOT NULL);
CREATE TABLE IF NOT EXISTS file_analysis(file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,signature TEXT NOT NULL,summary TEXT NOT NULL,category TEXT NOT NULL,tags TEXT NOT NULL,body TEXT NOT NULL,updated_at REAL NOT NULL);
CREATE TABLE IF NOT EXISTS media_properties(device TEXT NOT NULL,path TEXT NOT NULL,size INTEGER NOT NULL,mtime_ns INTEGER NOT NULL,duration_seconds REAL NOT NULL,checked_at REAL NOT NULL,PRIMARY KEY(device,path));
CREATE TABLE IF NOT EXISTS workspace_meta(device TEXT NOT NULL,path TEXT NOT NULL,owner TEXT NOT NULL DEFAULT '',tags TEXT NOT NULL DEFAULT '[]',status TEXT NOT NULL DEFAULT '',starred INTEGER NOT NULL DEFAULT 0,properties TEXT NOT NULL DEFAULT '{}',revision INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(device,path));
CREATE TABLE IF NOT EXISTS workspace_comments(id TEXT PRIMARY KEY,device TEXT NOT NULL,path TEXT NOT NULL,author TEXT NOT NULL,body TEXT NOT NULL,created REAL NOT NULL);
CREATE TABLE IF NOT EXISTS workspace_views(id TEXT PRIMARY KEY,name TEXT NOT NULL,kind TEXT NOT NULL,body TEXT NOT NULL,updated REAL NOT NULL);
CREATE TABLE IF NOT EXISTS workspace_properties(id TEXT PRIMARY KEY,name TEXT NOT NULL,kind TEXT NOT NULL,options TEXT NOT NULL DEFAULT '[]',revision INTEGER NOT NULL DEFAULT 1);
CREATE TABLE IF NOT EXISTS workspace_plans(id TEXT PRIMARY KEY,body TEXT NOT NULL,state TEXT NOT NULL,created REAL NOT NULL,receipt TEXT NOT NULL DEFAULT '{}');
CREATE TABLE IF NOT EXISTS folder_sync_pairs(id TEXT PRIMARY KEY,name TEXT NOT NULL,body TEXT NOT NULL,baseline TEXT NOT NULL DEFAULT '{}',revision INTEGER NOT NULL DEFAULT 1,last_sync REAL NOT NULL DEFAULT 0);
CREATE TABLE IF NOT EXISTS jobs(id TEXT PRIMARY KEY,kind TEXT NOT NULL,state TEXT NOT NULL,progress TEXT NOT NULL,created REAL NOT NULL,updated REAL NOT NULL);
CREATE TABLE IF NOT EXISTS extraction_cache(file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,signature TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS crawl_folders(device TEXT NOT NULL,root TEXT NOT NULL,interval_minutes INTEGER NOT NULL DEFAULT 0,analysis TEXT NOT NULL DEFAULT 'off',revision INTEGER NOT NULL DEFAULT 0,next_run REAL NOT NULL DEFAULT 0,last_started REAL NOT NULL DEFAULT 0,last_completed REAL NOT NULL DEFAULT 0,failures INTEGER NOT NULL DEFAULT 0,state TEXT NOT NULL DEFAULT 'new',report TEXT NOT NULL DEFAULT '{}',PRIMARY KEY(device,root));
CREATE TABLE IF NOT EXISTS analysis_attempts(file_id INTEGER PRIMARY KEY REFERENCES files(id) ON DELETE CASCADE,signature TEXT NOT NULL,attempts INTEGER NOT NULL,retry_after REAL NOT NULL,error TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS embedding_attempts(file_id INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,fingerprint TEXT NOT NULL,digest TEXT NOT NULL,retry_after REAL NOT NULL,error TEXT NOT NULL,PRIMARY KEY(file_id,fingerprint));
CREATE TABLE IF NOT EXISTS snapshot_revisions(fingerprint TEXT PRIMARY KEY,revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS background_state(id INTEGER PRIMARY KEY CHECK(id=1),paused INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO background_state VALUES(1,0);
CREATE VIRTUAL TABLE IF NOT EXISTS analysis_fts USING fts5(summary,category,tags,content='file_analysis',content_rowid='file_id',tokenize='unicode61 remove_diacritics 2');
CREATE TRIGGER IF NOT EXISTS analysis_insert AFTER INSERT ON file_analysis BEGIN
 INSERT INTO analysis_fts(rowid,summary,category,tags) VALUES(new.file_id,new.summary,new.category,new.tags);
 UPDATE revision SET value=value+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS analysis_delete AFTER DELETE ON file_analysis BEGIN
 INSERT INTO analysis_fts(analysis_fts,rowid,summary,category,tags) VALUES('delete',old.file_id,old.summary,old.category,old.tags);
 DELETE FROM passages WHERE file_id=old.file_id AND ordinal>=1000000;
 UPDATE revision SET value=value+1 WHERE id=1; END;
CREATE TRIGGER IF NOT EXISTS analysis_update AFTER UPDATE ON file_analysis BEGIN
 INSERT INTO analysis_fts(analysis_fts,rowid,summary,category,tags) VALUES('delete',old.file_id,old.summary,old.category,old.tags);
 INSERT INTO analysis_fts(rowid,summary,category,tags) VALUES(new.file_id,new.summary,new.category,new.tags);
 UPDATE revision SET value=value+1 WHERE id=1; END;

CREATE TABLE IF NOT EXISTS embedding_cache(fingerprint TEXT NOT NULL,content_hash TEXT NOT NULL,vector BLOB NOT NULL,dimensions INTEGER NOT NULL,PRIMARY KEY(fingerprint,content_hash));
CREATE INDEX IF NOT EXISTS passages_file_digest ON passages(file_id,digest);
