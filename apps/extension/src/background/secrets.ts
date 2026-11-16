// Secrets au repos (T5, note 0023) : chaque valeur est chiffrée en
// AES-256-GCM par une clé WebCrypto non exportable gardée dans IndexedDB,
// puis rangée dans chrome.storage.local sous `secret:<nom>`. Le nom est en
// donnée associée : une valeur déplacée sous un autre nom ne se déchiffre
// pas. Même principe que le Keystore Android (étape 3).

import { fromBase64, toBase64 } from './bytes';

/** Sous-ensemble de chrome.storage.local utilisé ici. */
export interface KeyValue {
  get(keys: string | string[] | null): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
  remove(keys: string | string[]): Promise<void>;
}

/** Fournit la clé de chiffrement des secrets. */
export interface KeySource {
  key(): Promise<CryptoKey>;
  /** Détruit la clé : les secrets déjà rangés deviennent illisibles. */
  forget(): Promise<void>;
}

const PREFIX = 'secret:';
const IV_LEN = 12;
const AAD_PREFIX = 'ot-secret-v1:';

function aad(name: string): Uint8Array<ArrayBuffer> {
  return new TextEncoder().encode(AAD_PREFIX + name);
}

export class SecretStore {
  constructor(
    private readonly kv: KeyValue,
    private readonly keys: KeySource,
  ) {}

  /** Toutes les valeurs rangées, déchiffrées. */
  async loadAll(): Promise<Map<string, Uint8Array>> {
    const items = await this.kv.get(null);
    const out = new Map<string, Uint8Array>();
    for (const [k, v] of Object.entries(items)) {
      if (k.startsWith(PREFIX) && typeof v === 'string') {
        const name = k.slice(PREFIX.length);
        out.set(name, await this.decrypt(name, v));
      }
    }
    return out;
  }

  async get(name: string): Promise<Uint8Array | undefined> {
    const items = await this.kv.get(PREFIX + name);
    const v = items[PREFIX + name];
    return typeof v === 'string' ? this.decrypt(name, v) : undefined;
  }

  async put(name: string, value: Uint8Array): Promise<void> {
    const key = await this.keys.key();
    const iv = crypto.getRandomValues(new Uint8Array(IV_LEN));
    const sealed = new Uint8Array(
      await crypto.subtle.encrypt(
        { name: 'AES-GCM', iv, additionalData: aad(name) },
        key,
        value as Uint8Array<ArrayBuffer>,
      ),
    );
    const blob = new Uint8Array(IV_LEN + sealed.length);
    blob.set(iv);
    blob.set(sealed, IV_LEN);
    await this.kv.set({ [PREFIX + name]: toBase64(blob) });
  }

  async delete(name: string): Promise<void> {
    await this.kv.remove(PREFIX + name);
  }

  /** Efface tous les secrets et la clé qui les chiffrait. */
  async clear(): Promise<void> {
    const items = await this.kv.get(null);
    await this.kv.remove(Object.keys(items).filter((k) => k.startsWith(PREFIX)));
    await this.keys.forget();
  }

  private async decrypt(name: string, stored: string): Promise<Uint8Array> {
    const blob = fromBase64(stored);
    if (blob.length <= IV_LEN) throw new Error(`secret ${name} illisible`);
    const key = await this.keys.key();
    try {
      return new Uint8Array(
        await crypto.subtle.decrypt(
          { name: 'AES-GCM', iv: blob.subarray(0, IV_LEN), additionalData: aad(name) },
          key,
          blob.subarray(IV_LEN),
        ),
      );
    } catch {
      // Clé détruite ou valeur altérée : le message ne dit rien du contenu.
      throw new Error(`secret ${name} illisible (clé de stockage perdue ou valeur altérée)`);
    }
  }
}

const DB_NAME = 'oldtown';
const DB_STORE = 'keys';
const KEY_ID = 'storage';

function request<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error ?? new Error('IndexedDB : erreur inconnue'));
  });
}

/**
 * Clé AES-GCM non exportable dans IndexedDB : le navigateur la garde sous
 * une forme que le JavaScript ne peut pas relire (structured clone d'une
 * CryptoKey non exportable).
 */
export class IndexedDbKeySource implements KeySource {
  private cached?: Promise<CryptoKey>;

  constructor(private readonly idb: IDBFactory = indexedDB) {}

  key(): Promise<CryptoKey> {
    this.cached ??= this.loadOrCreate().catch((e: unknown) => {
      this.cached = undefined;
      throw e;
    });
    return this.cached;
  }

  async forget(): Promise<void> {
    this.cached = undefined;
    const db = await this.open();
    try {
      const tx = db.transaction(DB_STORE, 'readwrite');
      await request(tx.objectStore(DB_STORE).delete(KEY_ID));
    } finally {
      db.close();
    }
  }

  private open(): Promise<IDBDatabase> {
    const req = this.idb.open(DB_NAME, 1);
    req.onupgradeneeded = () => req.result.createObjectStore(DB_STORE);
    return request(req);
  }

  private async loadOrCreate(): Promise<CryptoKey> {
    const db = await this.open();
    try {
      const existing = await request(
        db.transaction(DB_STORE, 'readonly').objectStore(DB_STORE).get(KEY_ID),
      );
      if (existing instanceof CryptoKey) return existing;
      const key = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, false, [
        'encrypt',
        'decrypt',
      ]);
      // `add` échoue si une autre instance vient de créer la sienne : on
      // relit alors celle-là, pour ne jamais en avoir deux.
      try {
        await request(db.transaction(DB_STORE, 'readwrite').objectStore(DB_STORE).add(key, KEY_ID));
        return key;
      } catch {
        const other = await request(
          db.transaction(DB_STORE, 'readonly').objectStore(DB_STORE).get(KEY_ID),
        );
        if (other instanceof CryptoKey) return other;
        throw new Error('IndexedDB : impossible de ranger la clé de stockage');
      }
    } finally {
      db.close();
    }
  }
}
