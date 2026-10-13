-- Removing membership cascades through its workspace bindings. Temporary
-- authorization receipts must not prevent that removal or survive reenrollment.
ALTER TABLE code_preflights
 DROP CONSTRAINT code_preflights_binding_id_fkey,
 ADD CONSTRAINT code_preflights_binding_id_fkey
 FOREIGN KEY(binding_id) REFERENCES workspace_bindings(id) ON DELETE CASCADE;
