-- Store the connection URI a driver uses to reach a data source.
ALTER TABLE data_sources ADD COLUMN connection_uri TEXT;
