-- Server receipt time, independent of producer clocks; old receipts use run time.
ALTER TABLE bundle_receipts ADD COLUMN received_at TEXT;
