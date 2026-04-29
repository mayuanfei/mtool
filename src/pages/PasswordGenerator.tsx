import { Copy, RefreshCcw, Check } from 'lucide-react';
import { useState, useEffect, useCallback } from 'react';
import { useI18n } from '../i18n';

const Toggle = ({ checked, onChange }: { checked: boolean; onChange: () => void }) => {
  return (
    <button
      onClick={onChange}
      className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors focus:outline-none focus:ring-2 focus:ring-indigo-500 focus:ring-offset-2 focus:ring-offset-slate-900 ${
        checked ? 'bg-indigo-500' : 'bg-slate-700'
      }`}
    >
      <span
        className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
          checked ? 'translate-x-6' : 'translate-x-1'
        }`}
      />
    </button>
  );
};

interface HistoryItem {
  id: string;
  timestamp: string;
  password: string;
  strength: 'STRONG' | 'GOOD' | 'FAIR' | 'WEAK';
}

const UPPERCASE = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';
const LOWERCASE = 'abcdefghijklmnopqrstuvwxyz';
const NUMBERS = '0123456789';
const SYMBOLS = '!@#$%^&*';

export function PasswordGenerator() {
  const { t } = useI18n();
  const [password, setPassword] = useState('');
  const [length, setLength] = useState(16);
  const [useUpper, setUseUpper] = useState(true);
  const [useLower, setUseLower] = useState(true);
  const [useNumbers, setUseNumbers] = useState(true);
  const [useSymbols, setUseSymbols] = useState(true);
  const [history, setHistory] = useState<HistoryItem[]>(() => {
    const saved = localStorage.getItem('mtool_pwd_history');
    if (saved) {
      try {
        return JSON.parse(saved);
      } catch (e) {
        return [];
      }
    }
    return [];
  });
  const [isCopied, setIsCopied] = useState(false);
  const [historyCopiedId, setHistoryCopiedId] = useState<string | null>(null);

  const calculateStrength = (): 'STRONG' | 'GOOD' | 'FAIR' | 'WEAK' => {
    let score = 0;
    if (useUpper) score += 1;
    if (useLower) score += 1;
    if (useNumbers) score += 1;
    if (useSymbols) score += 1;

    if (score === 4) return 'STRONG';
    if (score === 3) return 'GOOD';
    if (score === 2) return 'FAIR';
    return 'WEAK';
  };

  const generatePassword = useCallback((addToHistory = true) => {
    let charset = '';
    if (useUpper) charset += UPPERCASE;
    if (useLower) charset += LOWERCASE;
    if (useNumbers) charset += NUMBERS;
    if (useSymbols) charset += SYMBOLS;

    if (!charset) {
      setPassword('');
      return;
    }

    let generated = '';
    const array = new Uint32Array(length);
    window.crypto.getRandomValues(array);
    for (let i = 0; i < length; i++) {
      generated += charset[array[i] % charset.length];
    }

    setPassword(generated);

    if (addToHistory && generated) {
      const now = new Date();
      const pad = (n: number) => String(n).padStart(2, '0');
      const timeStr = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}:${pad(now.getSeconds())}`;
      
      setHistory(prev => {
        const newHistory = [
          {
            id: Math.random().toString(36).substring(7),
            timestamp: timeStr,
            password: generated,
            strength: calculateStrength()
          },
          ...prev
        ].slice(0, 5); // Keep last 5
        return newHistory;
      });
    }
  }, [length, useUpper, useLower, useNumbers, useSymbols]);

  // Initial generation
  useEffect(() => {
    generatePassword(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Save history to localStorage
  useEffect(() => {
    localStorage.setItem('mtool_pwd_history', JSON.stringify(history));
  }, [history]);

  // When length or options change, generate new one automatically (without adding to history for slider drag)
  useEffect(() => {
    generatePassword(false);
  }, [length, useUpper, useLower, useNumbers, useSymbols, generatePassword]);

  const handleCopy = async (text: string, isHistoryId: string | null = null) => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      if (isHistoryId) {
        setHistoryCopiedId(isHistoryId);
        setTimeout(() => setHistoryCopiedId(null), 2000);
      } else {
        setIsCopied(true);
        setTimeout(() => setIsCopied(false), 2000);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const strength = calculateStrength();
  
  const getStrengthColor = (s: string) => {
    if (s === 'STRONG') return 'bg-emerald-500';
    if (s === 'GOOD') return 'bg-blue-500';
    if (s === 'FAIR') return 'bg-amber-500';
    return 'bg-rose-500';
  };
  
  const getStrengthTextColor = (s: string) => {
    if (s === 'STRONG') return 'text-emerald-500';
    if (s === 'GOOD') return 'text-blue-500';
    if (s === 'FAIR') return 'text-amber-500';
    return 'text-rose-500';
  };

  const getStrengthBars = () => {
    let bars = 1;
    if (strength === 'FAIR') bars = 2;
    if (strength === 'GOOD') bars = 3;
    if (strength === 'STRONG') bars = 4;
    
    return (
      <div className="flex gap-2 flex-1 mr-4">
        {[1, 2, 3, 4].map(i => (
          <div 
            key={i} 
            className={`h-1.5 rounded-full flex-1 transition-colors ${
              i <= bars ? getStrengthColor(strength) : 'bg-slate-800'
            } ${i <= bars && strength === 'STRONG' ? 'shadow-[0_0_10px_rgba(16,185,129,0.5)]' : ''}`}
          ></div>
        ))}
      </div>
    );
  };

  const maskPassword = (pwd: string) => {
    if (pwd.length <= 4) return pwd;
    return '*'.repeat(8) + '_' + pwd.slice(-3);
  };

  return (
    <div className="max-w-4xl max-w-5xl mx-auto w-full">
      <div className="mb-10">
        <h1 className="text-4xl font-bold tracking-tight text-white mb-2">{t('Password Generator')}</h1>
        <p className="text-slate-400">{t('Generate secure, random passwords for your applications.')}</p>
      </div>

      <div className="space-y-6">
        
        {/* Main Generator Card */}
        <section className="bg-slate-900 border border-slate-800 rounded-xl p-6 shadow-2xl">
          
          {/* Password Display */}
          <div className="bg-slate-950 border border-slate-800 rounded-lg p-5 mb-8">
            <div className="flex items-center justify-between mb-6">
              <div className="text-3xl font-mono tracking-widest text-white overflow-hidden text-ellipsis whitespace-nowrap mr-4">
                {password || '---'}
              </div>
              <div className="flex items-center gap-3 shrink-0">
                <button 
                  onClick={() => generatePassword(true)}
                  className="p-2 text-slate-400 hover:text-white hover:bg-slate-800 rounded-lg transition-colors focus:outline-none"
                >
                  <RefreshCcw className="w-5 h-5" />
                </button>
                <button 
                  onClick={() => handleCopy(password)}
                  className="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white rounded-lg font-medium transition-colors flex items-center gap-2 focus:outline-none shadow-lg shadow-indigo-600/20"
                >
                  {isCopied ? <Check className="w-4 h-4" /> : <Copy className="w-4 h-4" />}
                  {isCopied ? t('Copied!') : t('Copy')}
                </button>
              </div>
            </div>
            
            <div className="flex items-center">
              {getStrengthBars()}
              <span className={`text-[11px] font-bold tracking-widest uppercase ${getStrengthTextColor(strength)}`}>
                {t(strength as any)}
              </span>
            </div>
          </div>

          {/* Controls */}
          <div className="mb-8">
            <div className="flex justify-between items-center mb-4">
              <span className="text-sm font-bold text-slate-300">{t('Password Length')}</span>
              <div className="flex items-center gap-3">
                <input 
                  type="number" 
                  min="4" max="128"
                  value={length}
                  onChange={(e) => {
                    let val = parseInt(e.target.value);
                    if (isNaN(val)) val = 4;
                    if (val > 128) val = 128;
                    setLength(val);
                  }}
                  onBlur={() => {
                    if (length < 4) setLength(4);
                  }}
                  className="w-16 bg-slate-950 border border-slate-700 text-slate-300 text-sm font-mono rounded px-2 py-1 focus:outline-none focus:border-indigo-500 text-center"
                />
              </div>
            </div>
            <input 
              type="range" min="4" max="128" step="1"
              value={length}
              onChange={(e) => setLength(parseInt(e.target.value))}
              className="w-full h-1.5 bg-slate-800 rounded-lg appearance-none cursor-pointer accent-indigo-500"
            />
          </div>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="bg-slate-950/50 border border-slate-800 rounded-lg p-4 flex items-center justify-between">
              <div>
                <p className="text-sm font-bold text-slate-300 mb-1">{t('Uppercase Letters')}</p>
                <p className="text-xs text-slate-500 font-mono">{t('A-Z')}</p>
              </div>
              <Toggle checked={useUpper} onChange={() => setUseUpper(!useUpper)} />
            </div>
            
            <div className="bg-slate-950/50 border border-slate-800 rounded-lg p-4 flex items-center justify-between">
              <div>
                <p className="text-sm font-bold text-slate-300 mb-1">{t('Lowercase Letters')}</p>
                <p className="text-xs text-slate-500 font-mono">{t('a-z')}</p>
              </div>
              <Toggle checked={useLower} onChange={() => setUseLower(!useLower)} />
            </div>

            <div className="bg-slate-950/50 border border-slate-800 rounded-lg p-4 flex items-center justify-between">
              <div>
                <p className="text-sm font-bold text-slate-300 mb-1">{t('Numbers')}</p>
                <p className="text-xs text-slate-500 font-mono">{t('0-9')}</p>
              </div>
              <Toggle checked={useNumbers} onChange={() => setUseNumbers(!useNumbers)} />
            </div>

            <div className="bg-slate-950/50 border border-slate-800 rounded-lg p-4 flex items-center justify-between">
              <div>
                <p className="text-sm font-bold text-slate-300 mb-1">{t('Symbols')}</p>
                <p className="text-xs text-slate-500 font-mono">{t('!@#$%^&*')}</p>
              </div>
              <Toggle checked={useSymbols} onChange={() => setUseSymbols(!useSymbols)} />
            </div>
          </div>
        </section>

        {/* History Table */}
        {history.length > 0 && (
          <section className="bg-slate-900 border border-slate-800 rounded-xl overflow-hidden shadow-2xl">
            <div className="overflow-x-auto">
              <table className="w-full text-left text-sm text-slate-400">
                <thead className="text-[10px] uppercase font-bold tracking-widest text-slate-500 border-b border-slate-800 bg-slate-950/50">
                  <tr>
                    <th className="px-6 py-4">{t('TIMESTAMP')}</th>
                    <th className="px-6 py-4">{t('PREVIEW')}</th>
                    <th className="px-6 py-4">{t('STRENGTH')}</th>
                    <th className="px-6 py-4 text-right">{t('ACTION')}</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-800">
                  {history.map((item) => (
                    <tr key={item.id} className="hover:bg-slate-800/30 transition-colors">
                      <td className="px-6 py-4 font-mono text-xs">{item.timestamp}</td>
                      <td className="px-6 py-4 font-mono text-slate-300">{maskPassword(item.password)}</td>
                      <td className="px-6 py-4">
                        <span className={`px-2 py-1 rounded text-[10px] font-bold uppercase ${
                          item.strength === 'STRONG' ? 'bg-emerald-500/10 text-emerald-400' :
                          item.strength === 'GOOD' ? 'bg-blue-500/10 text-blue-400' :
                          item.strength === 'FAIR' ? 'bg-amber-500/10 text-amber-400' :
                          'bg-rose-500/10 text-rose-400'
                        }`}>
                          {t(item.strength as any)}
                        </span>
                      </td>
                      <td className="px-6 py-4 text-right">
                        <button 
                          onClick={() => handleCopy(item.password, item.id)}
                          className="p-2 text-slate-500 hover:text-white hover:bg-slate-800 rounded transition-colors focus:outline-none inline-flex"
                        >
                          {historyCopiedId === item.id ? <Check className="w-4 h-4 text-emerald-400" /> : <Copy className="w-4 h-4" />}
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        )}

      </div>
    </div>
  );
}
