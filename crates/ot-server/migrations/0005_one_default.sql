-- Un seul appareil principal par personne (note 0079) : on garde le plus
-- ancien des appareils par défaut encore actifs ; un appareil révoqué
-- perd le sien. L'index le garantit ensuite.

UPDATE devices SET is_default = 0
 WHERE is_default = 1 AND revoked_at IS NULL
   AND id != (SELECT d.id FROM devices d
               WHERE d.user_id = devices.user_id AND d.revoked_at IS NULL
                 AND d.is_default = 1
               ORDER BY d.created_at, d.rowid LIMIT 1);

UPDATE devices SET is_default = 0 WHERE revoked_at IS NOT NULL;

CREATE UNIQUE INDEX devices_one_default ON devices (user_id)
 WHERE is_default = 1 AND revoked_at IS NULL;
