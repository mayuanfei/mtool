import { useState, useRef, useEffect } from 'react';
import { Lock, Copy, Trash2, ArrowDown, ShieldAlert, Check, ChevronDown } from 'lucide-react';
import { useI18n } from '../i18n';
import CryptoJS from 'crypto-js';
import { sm2, sm3, sm4 } from 'sm-crypto';
import JSEncrypt from 'jsencrypt';
import { invoke } from '@tauri-apps/api/core';

type Category = 'hash' | 'symmetric' | 'asymmetric' | 'hq';
type Algorithm = 'MD5' | 'SHA1' | 'SHA256' | 'SM3' | 'AES' | 'DES' | 'SM4' | 'RSA' | 'SM2' | 'HQ_DLL';
type DataFormat = 'UTF8' | 'HEX' | 'BASE64';

function CustomSelect({ options, value, onChange, disabled, className, menuClassName }: any) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  
  useEffect(() => {
    const handleClickOutside = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handleClickOutside);
    return () => document.removeEventListener('mousedown', handleClickOutside);
  }, []);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen(!open)}
        className={`flex items-center justify-between gap-2 bg-transparent outline-none ${disabled ? 'opacity-50 cursor-not-allowed' : 'cursor-pointer'} ${className}`}
      >
        <span>{options.find((o: any) => o.value === value)?.label || value}</span>
        <ChevronDown className="w-3 h-3 opacity-60" />
      </button>
      {open && !disabled && (
        <div className={`absolute z-50 mt-1 th-bg-surface border th-border rounded-lg shadow-xl overflow-hidden py-1 ${menuClassName || 'w-full left-0 min-w-[8rem]'}`}>
          {options.map((o: any) => (
            <button
              key={o.value}
              type="button"
              className={`w-full text-left px-3 py-2 text-sm transition-colors hover:bg-indigo-500/10 ${value === o.value ? 'text-indigo-400 font-medium' : 'th-text'}`}
              onClick={() => { onChange(o.value); setOpen(false); }}
            >
              {o.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function CryptoTool() {
  const { t } = useI18n();
  const [category, setCategory] = useState<Category>('hash');
  const [algorithm, setAlgorithm] = useState<Algorithm>('MD5');

  const [input, setInput] = useState('');
  const [output, setOutput] = useState('');
  const [cryptoKey, setCryptoKey] = useState('');
  const [iv, setIv] = useState('');
  const [mode, setMode] = useState('CBC');
  const [padding, setPadding] = useState('Pkcs7');

  const [inputFormat, setInputFormat] = useState<DataFormat>('UTF8');
  const [outputFormat, setOutputFormat] = useState<DataFormat>('BASE64');
  const [keyFormat, setKeyFormat] = useState<DataFormat>('UTF8');
  const [ivFormat, setIvFormat] = useState<DataFormat>('UTF8');
  
  // For Asymmetric
  const [publicKey, setPublicKey] = useState('');
  const [privateKey, setPrivateKey] = useState('');

  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const categories: { value: Category; label: string }[] = [
    { value: 'hash', label: t('Hash / Digest') },
    { value: 'symmetric', label: t('Symmetric') },
    { value: 'asymmetric', label: t('Asymmetric') },
    { value: 'hq', label: t('HQ DLL') },
  ];

  const algorithms: Record<Category, Algorithm[]> = {
    hash: ['MD5', 'SHA1', 'SHA256', 'SM3'],
    symmetric: ['AES', 'DES', 'SM4'],
    asymmetric: ['RSA', 'SM2'],
    hq: ['HQ_DLL'],
  };

  const handleCategoryChange = (c: Category) => {
    setCategory(c);
    setAlgorithm(algorithms[c][0]);
    setOutput('');
    setError(null);
  };

  const handleAlgorithmChange = (a: Algorithm) => {
    setAlgorithm(a);
    setOutput('');
    setError(null);
  };

  const getCryptoJsMode = () => {
    return mode === 'ECB' ? CryptoJS.mode.ECB : CryptoJS.mode.CBC;
  };

  const getCryptoJsPadding = () => {
    if (padding === 'ZeroPadding') return CryptoJS.pad.ZeroPadding;
    if (padding === 'NoPadding') return CryptoJS.pad.NoPadding;
    return CryptoJS.pad.Pkcs7;
  };

  const parseData = (data: string, format: DataFormat) => {
    if (format === 'HEX') return CryptoJS.enc.Hex.parse(data);
    if (format === 'BASE64') return CryptoJS.enc.Base64.parse(data);
    return CryptoJS.enc.Utf8.parse(data);
  };

  const stringifyData = (wordArray: any, format: DataFormat) => {
    if (format === 'HEX') return CryptoJS.enc.Hex.stringify(wordArray);
    if (format === 'BASE64') return CryptoJS.enc.Base64.stringify(wordArray);
    return CryptoJS.enc.Utf8.stringify(wordArray);
  };

  const toHex = (data: string, format: DataFormat) => {
    return CryptoJS.enc.Hex.stringify(parseData(data, format));
  };
  const fromHex = (hexData: string, format: DataFormat) => {
    return stringifyData(CryptoJS.enc.Hex.parse(hexData), format);
  };

  const handleAction = async (isEncrypt: boolean) => {
    setError(null);
    
    // Provide visual feedback for recalculation by temporarily clearing the output
    if (output) {
      setOutput('');
      await new Promise((resolve) => setTimeout(resolve, 50));
    }

    try {
      let res = '';
      
      // ================= HASH =================
      if (category === 'hash') {
        if (!isEncrypt) throw new Error(t('Hash algorithms cannot be decrypted.'));
        const inputWa = parseData(input, inputFormat);
        let hashWa;
        if (algorithm === 'MD5') hashWa = CryptoJS.MD5(inputWa);
        else if (algorithm === 'SHA1') hashWa = CryptoJS.SHA1(inputWa);
        else if (algorithm === 'SHA256') hashWa = CryptoJS.SHA256(inputWa);
        else if (algorithm === 'SM3') {
          // SM3 in sm-crypto takes a string or array, but we can pass hex string via config
          const hexIn = toHex(input, inputFormat);
          hashWa = CryptoJS.enc.Hex.parse(sm3(hexIn, { encoding: 'hex' } as any));
        }
        res = stringifyData(hashWa, outputFormat);
      } 
      // ================= SYMMETRIC =================
      else if (category === 'symmetric') {
        if (!cryptoKey) throw new Error(t('Key is required.'));
        
        if (algorithm === 'AES' || algorithm === 'DES') {
          const cfg: any = { mode: getCryptoJsMode(), padding: getCryptoJsPadding() };
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            cfg.iv = parseData(iv, ivFormat);
          }
          const keyWa = parseData(cryptoKey, keyFormat);
          const inputWa = parseData(input, inputFormat);
          
          let engine = algorithm === 'AES' ? CryptoJS.AES : CryptoJS.DES;

          if (isEncrypt) {
            const encrypted = engine.encrypt(inputWa, keyWa, cfg);
            res = outputFormat === 'HEX' ? encrypted.ciphertext.toString(CryptoJS.enc.Hex)
                : outputFormat === 'UTF8' ? encrypted.ciphertext.toString(CryptoJS.enc.Utf8)
                : encrypted.toString();
          } else {
            // For decryption, CryptoJS takes Base64 string or CipherParams
            let decryptInput = input;
            if (inputFormat === 'HEX' || inputFormat === 'UTF8') {
              decryptInput = CryptoJS.enc.Base64.stringify(parseData(input, inputFormat));
            }
            const decrypted = engine.decrypt(decryptInput, keyWa, cfg);
            res = stringifyData(decrypted, outputFormat);
            if (!res) throw new Error(t('Decryption failed. Invalid Key/IV or data.'));
          }
        } 
        else if (algorithm === 'SM4') {
          const keyHex = toHex(cryptoKey, keyFormat);
          const cfg: any = {};
          if (mode === 'CBC') {
            if (!iv) throw new Error(t('IV is required for CBC mode.'));
            cfg.iv = toHex(iv, ivFormat);
            cfg.mode = 'cbc';
          }
          
          const inputHex = toHex(input, inputFormat);
          // convert hex to byte array for sm-crypto
          const hexToBytes = (hex: string) => {
            let bytes = [];
            for (let c = 0; c < hex.length; c += 2) bytes.push(parseInt(hex.substr(c, 2), 16));
            return bytes;
          };

          if (isEncrypt) {
            const outHex = sm4.encrypt(hexToBytes(inputHex), keyHex, cfg);
            res = fromHex(outHex, outputFormat);
          } else {
            const outHex = sm4.decrypt(hexToBytes(inputHex), keyHex, cfg);
            res = fromHex(outHex, outputFormat);
          }
        }
      }
      // ================= ASYMMETRIC =================
      else if (category === 'asymmetric') {
        if (algorithm === 'RSA') {
          const encryptor = new JSEncrypt();
          if (isEncrypt) {
            if (!publicKey) throw new Error(t('Public Key is required for encryption.'));
            encryptor.setPublicKey(publicKey);
            const encoded = encryptor.encrypt(input);
            if (!encoded) throw new Error(t('RSA Encryption failed.'));
            res = encoded;
          } else {
            if (!privateKey) throw new Error(t('Private Key is required for decryption.'));
            encryptor.setPrivateKey(privateKey);
            const decoded = encryptor.decrypt(input);
            if (!decoded) throw new Error(t('RSA Decryption failed.'));
            res = decoded;
          }
        }
        else if (algorithm === 'SM2') {
          if (isEncrypt) {
            if (!publicKey) throw new Error(t('Public Key is required for encryption.'));
            res = sm2.doEncrypt(input, publicKey, 1); // 1 for cipherMode C1C3C2
          } else {
            if (!privateKey) throw new Error(t('Private Key is required for decryption.'));
            res = sm2.doDecrypt(input, privateKey, 1, { output: 'utf8' as any });
          }
        }
      }
      // ================= HQ DLL =================
      else if (category === 'hq') {
        // Call Rust Backend Command
        try {
          res = await invoke('hq_crypto', { 
            action: isEncrypt ? 'encrypt' : 'decrypt',
            payload: input 
          });
        } catch (err: any) {
          throw new Error(`DLL Call Error: ${err}`);
        }
      }

      setOutput(res);
    } catch (err: any) {
      setError(err.message || String(err));
    }
  };

  const copyOutput = async () => {
    if (!output) return;
    try {
      await navigator.clipboard.writeText(output);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy', err);
    }
  };

  const generateKeys = () => {
    if (algorithm === 'RSA') {
      const encryptor = new JSEncrypt({ default_key_size: '1024' });
      encryptor.getKey();
      setPublicKey(encryptor.getPublicKey());
      setPrivateKey(encryptor.getPrivateKey());
    } else if (algorithm === 'SM2') {
      const keypair = sm2.generateKeyPairHex();
      setPublicKey(keypair.publicKey);
      setPrivateKey(keypair.privateKey);
    }
  };

  return (
    <div className="w-full h-full flex flex-col min-h-0">
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4 shrink-0">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <Lock className="w-5 h-5 text-indigo-400" />
          {t('Crypto Tool')}
        </h2>
      </div>

      <div className="flex-1 flex flex-col gap-4 min-h-0">
        {/* Top Navigation */}
        <div className="flex flex-wrap gap-2 flex-shrink-0 bg-indigo-500/5 p-1 rounded-lg border th-border">
          {categories.map((c) => (
            <button
              key={c.value}
              onClick={() => handleCategoryChange(c.value)}
              className={`px-4 py-2 rounded-md text-sm font-medium transition-colors ${
                category === c.value 
                  ? 'bg-indigo-600 text-white shadow-sm' 
                  : 'th-text-2 hover:bg-indigo-500/10'
              }`}
            >
              {c.label}
            </button>
          ))}
        </div>

        {/* Algorithm Selector */}
        <div className="flex flex-wrap gap-2 flex-shrink-0">
          {algorithms[category].map((a) => (
            <button
              key={a}
              onClick={() => handleAlgorithmChange(a)}
              className={`px-3 py-1.5 rounded-md text-xs font-medium transition-colors border ${
                algorithm === a 
                  ? 'bg-indigo-500/20 text-indigo-400 border-indigo-500/30' 
                  : 'th-bg-surface th-text-3 border-transparent hover:border-indigo-500/20'
              }`}
            >
              {a}
            </button>
          ))}
        </div>

        {/* Configuration Area */}
        {category === 'symmetric' && (
          <div className="grid grid-cols-4 gap-4 shrink-0 border th-border rounded-xl p-4 th-bg-surface-h">
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('Mode')}</label>
              <CustomSelect 
                value={mode}
                onChange={(val: string) => setMode(val)}
                options={[{ value: 'CBC', label: 'CBC' }, { value: 'ECB', label: 'ECB' }]}
                className="w-full px-3 py-2 border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500"
              />
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <label className="text-xs font-semibold th-text-muted uppercase">{t('Padding')}</label>
              <CustomSelect 
                value={padding}
                onChange={(val: string) => setPadding(val)}
                options={[{ value: 'Pkcs7', label: 'PKCS7' }, { value: 'ZeroPadding', label: 'ZeroPadding' }, { value: 'NoPadding', label: 'NoPadding' }]}
                className="w-full px-3 py-2 border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500"
              />
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <div className="flex justify-between items-center">
                <label className="text-xs font-semibold th-text-muted uppercase">{t('Key')}</label>
                <CustomSelect 
                  value={keyFormat}
                  onChange={(val: DataFormat) => setKeyFormat(val)}
                  options={[{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]}
                  className="text-[10px] th-text-3 hover:text-indigo-400 font-medium"
                  menuClassName="right-0 min-w-[6rem]"
                />
              </div>
              <input 
                type="text"
                value={cryptoKey}
                onChange={(e) => setCryptoKey(e.target.value)}
                placeholder="Secret Key"
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none placeholder:text-gray-500/40"
              />
            </div>
            <div className="flex flex-col gap-1.5 col-span-1">
              <div className="flex justify-between items-center">
                <label className="text-xs font-semibold th-text-muted uppercase">{t('IV')}</label>
                <CustomSelect 
                  value={ivFormat}
                  onChange={(val: DataFormat) => setIvFormat(val)}
                  disabled={mode === 'ECB'}
                  options={[{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]}
                  className="text-[10px] th-text-3 hover:text-indigo-400 font-medium"
                  menuClassName="right-0 min-w-[6rem]"
                />
              </div>
              <input 
                type="text"
                value={iv}
                onChange={(e) => setIv(e.target.value)}
                disabled={mode === 'ECB'}
                placeholder={mode === 'ECB' ? t('Not needed for ECB') : 'Initialization Vector'}
                className="w-full px-3 py-2 bg-transparent border th-border rounded-lg th-text text-sm focus:ring-1 focus:ring-indigo-500 outline-none disabled:opacity-50 placeholder:text-gray-500/40"
              />
            </div>
          </div>
        )}

        {category === 'asymmetric' && (
          <div className="flex flex-col gap-4 shrink-0 border th-border rounded-xl p-4 th-bg-surface-h">
            <div className="flex justify-between items-center">
              <span className="text-xs font-semibold th-text-muted uppercase">{t('Keys Configuration')}</span>
              <button 
                onClick={generateKeys}
                className="text-xs text-indigo-400 hover:text-indigo-300 font-medium"
              >
                {t('Auto Generate Key Pair')}
              </button>
            </div>
            <div className="grid grid-cols-2 gap-4">
              <div className="flex flex-col gap-1.5 col-span-1">
                <label className="text-xs th-text-3">{t('Public Key')}</label>
                <textarea 
                  value={publicKey}
                  onChange={(e) => setPublicKey(e.target.value)}
                  placeholder="-----BEGIN PUBLIC KEY-----..."
                  className="w-full h-24 p-3 bg-transparent border th-border rounded-lg th-text text-xs font-mono resize-none focus:ring-1 focus:ring-indigo-500 outline-none placeholder:text-gray-500/40"
                />
              </div>
              <div className="flex flex-col gap-1.5 col-span-1">
                <label className="text-xs th-text-3">{t('Private Key')}</label>
                <textarea 
                  value={privateKey}
                  onChange={(e) => setPrivateKey(e.target.value)}
                  placeholder="-----BEGIN PRIVATE KEY-----..."
                  className="w-full h-24 p-3 bg-transparent border th-border rounded-lg th-text text-xs font-mono resize-none focus:ring-1 focus:ring-indigo-500 outline-none placeholder:text-gray-500/40"
                />
              </div>
            </div>
          </div>
        )}

        {category === 'hq' && (
          <div className="flex items-center gap-3 shrink-0 border border-indigo-500/20 bg-indigo-500/5 rounded-xl p-4 text-sm text-indigo-400">
            <ShieldAlert className="w-5 h-5 flex-shrink-0" />
            <span>{t('This feature requires the HQ DLL installed in C:\\Windows\\System32. Only available on Windows environments.')}</span>
          </div>
        )}

        <div className="flex-1 flex gap-4 min-h-0">
          
          {/* Input Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <div className="flex items-center gap-3">
                <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Input')}</span>
                <CustomSelect 
                  value={inputFormat}
                  onChange={(val: DataFormat) => setInputFormat(val)}
                  options={[{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]}
                  className="text-xs th-text-3 hover:text-indigo-400 font-medium bg-indigo-500/5 px-2 py-1 rounded"
                  menuClassName="left-0 min-w-[6rem]"
                />
              </div>
              <button 
                onClick={() => { setInput(''); setOutput(''); setError(null); }}
                className="p-1.5 th-text-muted hover:text-rose-400 hover:bg-rose-500/10 rounded transition-colors"
                title={t('Clear Input')}
              >
                <Trash2 className="w-4 h-4" />
              </button>
            </div>
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              className="flex-1 w-full p-4 bg-transparent border-none focus:ring-0 th-text placeholder:text-gray-500/40 resize-none font-mono text-sm leading-relaxed"
              placeholder="..."
              spellCheck={false}
            />
          </div>

          {/* Action Center */}
          <div className="flex flex-col justify-center gap-4 shrink-0 px-2">
            <button
              onClick={() => handleAction(true)}
              className="flex flex-col items-center justify-center gap-1 min-w-[6rem] px-3 py-3 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg transition-all active:scale-95 text-center"
            >
              <span className="font-bold text-sm">
                {category === 'hash' ? t('Hash') : category === 'asymmetric' ? t('Public Key Encrypt') : t('Encrypt')}
              </span>
              <ArrowDown className="w-4 h-4 -rotate-90" />
            </button>
            {category !== 'hash' && (
              <button
                onClick={() => handleAction(false)}
                className="flex flex-col items-center justify-center gap-1 min-w-[6rem] px-3 py-3 th-bg-surface th-hover-surface border th-border th-text-2 rounded-xl shadow-sm transition-all active:scale-95 text-center"
              >
                <ArrowDown className="w-4 h-4 -rotate-90" />
                <span className="font-bold text-sm">
                  {category === 'asymmetric' ? t('Private Key Decrypt') : t('Decrypt')}
                </span>
              </button>
            )}
          </div>

          {/* Output Panel */}
          <div className="flex-1 flex flex-col min-h-0 border th-border rounded-xl overflow-hidden shadow-sm th-bg-card">
            <div className="px-4 py-3 border-b th-border th-bg-surface-h flex items-center justify-between">
              <div className="flex items-center gap-3">
                <span className="font-semibold text-sm th-text-2 uppercase tracking-tight">{t('Output')}</span>
                <CustomSelect 
                  value={outputFormat}
                  onChange={(val: DataFormat) => setOutputFormat(val)}
                  options={[{ value: 'UTF8', label: t('UTF8') }, { value: 'HEX', label: t('HEX') }, { value: 'BASE64', label: t('BASE64') }]}
                  className="text-xs th-text-3 hover:text-indigo-400 font-medium bg-indigo-500/5 px-2 py-1 rounded"
                  menuClassName="left-0 min-w-[6rem]"
                />
              </div>
              <button 
                onClick={copyOutput}
                className={`p-1.5 rounded transition-colors ${copied ? 'text-emerald-400 bg-emerald-500/10' : 'th-text-muted hover:text-emerald-400 hover:bg-emerald-500/10'}`}
                title={copied ? t('Copied!') : t('Copy Output')}
              >
                {copied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
              </button>
            </div>
            <div className="flex-1 relative">
              <textarea
                value={output}
                readOnly
                className="absolute inset-0 w-full h-full p-4 bg-transparent border-none focus:ring-0 th-text placeholder:text-gray-500/40 resize-none font-mono text-sm leading-relaxed"
                placeholder="..."
                spellCheck={false}
              />
              {error && (
                <div className="absolute bottom-4 left-4 right-4 p-3 bg-rose-500/10 border border-rose-500/20 text-rose-400 text-sm rounded-lg backdrop-blur-sm shadow-sm break-words">
                  {error}
                </div>
              )}
            </div>
          </div>
          
        </div>
      </div>
    </div>
  );
}
