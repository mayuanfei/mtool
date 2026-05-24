import { useState, useEffect, useRef } from 'react';
import { Network, Send, Download, History, UserPlus, Trash2, FolderOpen, FileCheck, XCircle, Copy, Check, RefreshCw, Eye, Sparkles, Edit3 } from 'lucide-react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useI18n } from '../i18n';
import { CustomSelect } from '../components/CustomSelect';

interface PeerInfo {
  ip: string;
  port: number;
  hostname: string;
  alias: string;
  added_at: number;
}

interface Transmission {
  transferId: string;
  direction: 'send' | 'recv';
  filename: string;
  filesize: number;
  peerName: string;
  peerIp?: string;
  progress: number;
  bytesTransferred: number;
  speed: number;
  status: 'transferring' | 'success' | 'failed' | 'rejected';
}

interface HistoryRecord {
  id: string;
  direction: 'send' | 'recv';
  filename: string;
  filesize: number;
  peerName: string;
  peerIp: string;
  status: 'success' | 'failed' | 'rejected' | 'cancelled';
  timestamp: number;
  savePath?: string;
}

interface ConfirmConfig {
  title: string;
  message: string;
  onConfirm: () => void | Promise<void>;
  onCancel?: () => void;
}

interface PendingRequest {
  request_id: string;
  sender_ip: string;
  sender_name: string;
}

export function FileTransfer() {
  const { t } = useI18n();

  // Local device info
  const [localIps, setLocalIps] = useState<string[]>([]);
  const [localPort, setLocalPort] = useState<number>(0);
  const [localHostname, setLocalHostname] = useState<string>('');

  // Config
  const [saveDir, setSaveDir] = useState<string | null>(null);
  const [trustedPeers, setTrustedPeers] = useState<PeerInfo[]>([]);

  // Form states
  const [peerIpInput, setPeerIpInput] = useState('');
  const [peerAliasInput, setPeerAliasInput] = useState('');
  const [addingPeer, setAddingPeer] = useState(false);
  const [addMessage, setAddMessage] = useState<{ text: string; isError: boolean } | null>(null);

  // File selection
  const [selectedFile, setSelectedFile] = useState<{ path: string; name: string; size: number } | null>(null);
  const [targetPeerKey, setTargetPeerKey] = useState('');

  // Active transmissions and history
  const [transmissions, setTransmissions] = useState<Record<string, Transmission>>({});
  const [history, setHistory] = useState<HistoryRecord[]>([]);

  // Overlay request modal
  const [pendingFriendRequest, setPendingFriendRequest] = useState<PendingRequest | null>(null);

  // Copy state
  const [copiedIp, setCopiedIp] = useState<string | null>(null);

  // Custom confirmation modal state
  const [confirmConfig, setConfirmConfig] = useState<ConfirmConfig | null>(null);

  // Peer editing states
  const [editingPeerKey, setEditingPeerKey] = useState<string | null>(null);
  const [editAliasInput, setEditAliasInput] = useState('');

  // Toast state
  const [toastMessage, setToastMessage] = useState<{ text: string; isError: boolean } | null>(null);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const showToast = (text: string, isError: boolean = false) => {
    if (toastTimerRef.current) {
      clearTimeout(toastTimerRef.current);
    }
    setToastMessage({ text, isError });
    toastTimerRef.current = setTimeout(() => {
      setToastMessage(null);
    }, 4000);
  };

  // Fetch local info and config on mount
  useEffect(() => {
    loadLocalInfo();
    loadPeersAndConfig();
    loadHistory();
    return () => {
      if (toastTimerRef.current) {
        clearTimeout(toastTimerRef.current);
      }
    };
  }, []);

  const loadLocalInfo = async () => {
    try {
      const info = await invoke<{ ips: string[]; port: number; hostname: string }>('get_local_transfer_info');
      setLocalIps(info.ips);
      setLocalPort(info.port);
      setLocalHostname(info.hostname);
    } catch (e) {
      console.error('Failed to load local transfer info:', e);
    }
  };

  const loadPeersAndConfig = async () => {
    try {
      const config = await invoke<{ save_dir: string | null; trusted_peers: PeerInfo[] }>('get_transfer_config');
      setSaveDir(config.save_dir);
      setTrustedPeers(config.trusted_peers);
    } catch (e) {
      console.error('Failed to load transfer config:', e);
    }
  };

  const loadHistory = () => {
    try {
      const saved = localStorage.getItem('mtool_transfer_history');
      if (saved) {
        const parsed = JSON.parse(saved) as HistoryRecord[];
        parsed.sort((a, b) => b.timestamp - a.timestamp);
        setHistory(parsed);
      } else {
        setHistory([]);
      }
    } catch (e) {
      console.error('Failed to load transfer history:', e);
    }
  };

  const saveHistoryRecord = (record: HistoryRecord) => {
    let currentHistory: HistoryRecord[] = [];
    try {
      const saved = localStorage.getItem('mtool_transfer_history');
      if (saved) {
        currentHistory = JSON.parse(saved) as HistoryRecord[];
      }
    } catch (e) {
      console.error('Failed to parse history during save:', e);
    }

    // Avoid duplicate records by checking both id and direction
    if (currentHistory.some(r => r.id === record.id && r.direction === record.direction)) {
      return;
    }

    const next = [record, ...currentHistory];
    next.sort((a, b) => b.timestamp - a.timestamp);
    const sliced = next.slice(0, 100); // Limit to 100 entries
    localStorage.setItem('mtool_transfer_history', JSON.stringify(sliced));
    setHistory(sliced);
  };

  // Setup Tauri Event Listeners (handled safely to avoid React StrictMode duplicate registrations)
  useEffect(() => {
    let active = true;
    let localUnlistens: (() => void)[] = [];

    const setup = async () => {
      try {
        const listeners = [
          // 1. Friend request listener
          listen<PendingRequest>('friend-request', (event) => {
            setPendingFriendRequest(event.payload);
          }),

          // 2. Trusted peers updated listener
          listen('trusted-peers-updated', () => {
            loadPeersAndConfig();
          }),

          // 3. Receive file started listener
          listen<{
            transfer_id: string;
            filename: string;
            filesize: number;
            sender_name: string;
            sender_ip: string;
            save_path: string;
          }>('recv-started', (event) => {
            const { transfer_id, filename, filesize, sender_name, sender_ip } = event.payload;
            setTransmissions(prev => ({
              ...prev,
              [transfer_id]: {
                transferId: transfer_id,
                direction: 'recv',
                filename,
                filesize,
                peerName: sender_name,
                peerIp: sender_ip,
                progress: 0,
                bytesTransferred: 0,
                speed: 0,
                status: 'transferring',
              }
            }));
          }),

          // 4. Receive file progress listener
          listen<{
            transfer_id: string;
            bytes_received: number;
            total_bytes: number;
            speed: number;
          }>('recv-progress', (event) => {
            const { transfer_id, bytes_received, total_bytes, speed } = event.payload;
            const progress = total_bytes > 0 ? Math.round((bytes_received / total_bytes) * 100) : 0;
            setTransmissions(prev => {
              if (!prev[transfer_id]) return prev;
              return {
                ...prev,
                [transfer_id]: {
                  ...prev[transfer_id],
                  progress,
                  bytesTransferred: bytes_received,
                  speed,
                  status: 'transferring',
                }
              };
            });
          }),

          // 5. Receive file success listener
          listen<{
            transfer_id: string;
            save_path: string;
          }>('recv-success', (event) => {
            const { transfer_id, save_path } = event.payload;
            setTransmissions(prev => {
              const tx = prev[transfer_id];
              if (tx) {
                saveHistoryRecord({
                  id: transfer_id,
                  direction: 'recv',
                  filename: tx.filename,
                  filesize: tx.filesize,
                  peerName: tx.peerName,
                  peerIp: tx.peerIp || '',
                  status: 'success',
                  timestamp: Date.now(),
                  savePath: save_path,
                });
              }
              const next = { ...prev };
              delete next[transfer_id];
              return next;
            });
          }),

          // 6. Send started listener
          listen<{
            transfer_id: string;
            filename: string;
            filesize: number;
          }>('send-started', (event) => {
            const { transfer_id, filename, filesize } = event.payload;
            setTransmissions(prev => ({
              ...prev,
              [transfer_id]: {
                transferId: transfer_id,
                direction: 'send',
                filename,
                filesize,
                peerName: '',
                progress: 0,
                bytesTransferred: 0,
                speed: 0,
                status: 'transferring',
              }
            }));
          }),

          // 7. Send progress listener
          listen<{
            transfer_id: string;
            bytes_sent: number;
            total_bytes: number;
            speed: number;
          }>('send-progress', (event) => {
            const { transfer_id, bytes_sent, total_bytes, speed } = event.payload;
            const progress = total_bytes > 0 ? Math.round((bytes_sent / total_bytes) * 100) : 0;
            setTransmissions(prev => {
              if (!prev[transfer_id]) return prev;
              return {
                ...prev,
                [transfer_id]: {
                  ...prev[transfer_id],
                  progress,
                  bytesTransferred: bytes_sent,
                  speed,
                  status: 'transferring',
                }
              };
            });
          }),

          // 8. Send success listener
          listen<{ transfer_id: string }>('send-success', (event) => {
            const { transfer_id } = event.payload;
            setTransmissions(prev => {
              const tx = prev[transfer_id];
              if (tx) {
                saveHistoryRecord({
                  id: transfer_id,
                  direction: 'send',
                  filename: tx.filename,
                  filesize: tx.filesize,
                  peerName: tx.peerName || 'Receiver',
                  peerIp: tx.peerIp || '',
                  status: 'success',
                  timestamp: Date.now(),
                });
              }
              const next = { ...prev };
              delete next[transfer_id];
              return next;
            });
          }),

          // 9. Send error listener
          listen<{ transfer_id: string; error_message: string }>('send-error', (event) => {
            const { transfer_id, error_message } = event.payload;
            setTransmissions(prev => {
              const tx = prev[transfer_id];
              if (tx) {
                saveHistoryRecord({
                  id: transfer_id,
                  direction: 'send',
                  filename: tx.filename,
                  filesize: tx.filesize,
                  peerName: tx.peerName || 'Receiver',
                  peerIp: tx.peerIp || '',
                  status: 'failed',
                  timestamp: Date.now(),
                });
              }
              const next = { ...prev };
              delete next[transfer_id];
              return next;
            });
            showToast(t('Failed') + `: ${error_message}`, true);
          }),

          // 10. Send rejected listener
          listen<{ transfer_id: string; reason: string }>('send-rejected', (event) => {
            const { transfer_id, reason } = event.payload;
            setTransmissions(prev => {
              const tx = prev[transfer_id];
              if (tx) {
                saveHistoryRecord({
                  id: transfer_id,
                  direction: 'send',
                  filename: tx.filename,
                  filesize: tx.filesize,
                  peerName: tx.peerName || 'Receiver',
                  peerIp: tx.peerIp || '',
                  status: 'rejected',
                  timestamp: Date.now(),
                });
              }
              const next = { ...prev };
              delete next[transfer_id];
              return next;
            });
            showToast(t('Friend Request Rejected') + `: ${reason}`, true);
          }),

          // 11. Receive error listener
          listen<{ transfer_id: string; error_message: string }>('recv-error', (event) => {
            const { transfer_id, error_message } = event.payload;
            setTransmissions(prev => {
              const tx = prev[transfer_id];
              if (tx) {
                saveHistoryRecord({
                  id: transfer_id,
                  direction: 'recv',
                  filename: tx.filename,
                  filesize: tx.filesize,
                  peerName: tx.peerName || 'Sender',
                  peerIp: tx.peerIp || '',
                  status: 'failed',
                  timestamp: Date.now(),
                });
              }
              const next = { ...prev };
              delete next[transfer_id];
              return next;
            });
            showToast(`${t('Receive failed')}: ${error_message}`, true);
          })
        ];

        const resolvedUnlistens = await Promise.all(listeners);
        
        if (!active) {
          resolvedUnlistens.forEach(fn => fn());
        } else {
          localUnlistens = resolvedUnlistens;
        }
      } catch (e) {
        console.error('Failed to setup Tauri listeners:', e);
      }
    };

    setup();

    return () => {
      active = false;
      localUnlistens.forEach(fn => fn());
    };
  }, []);

  // Synchronize history across different instances/tabs sharing localStorage
  useEffect(() => {
    const handleStorageChange = (e: StorageEvent) => {
      if (e.key === 'mtool_transfer_history') {
        loadHistory();
      }
    };
    window.addEventListener('storage', handleStorageChange);
    return () => {
      window.removeEventListener('storage', handleStorageChange);
    };
  }, []);

  // Helper formatting functions
  const formatBytes = (bytes: number, decimals = 1) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
  };

  const formatSpeed = (bytesPerSec: number) => {
    return formatBytes(bytesPerSec, 1) + '/s';
  };

  // Actions
  const handleSelectSaveDir = async () => {
    try {
      const path = await invoke<string | null>('select_save_dir');
      if (path) {
        setSaveDir(path);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleResetSaveDir = async () => {
    try {
      await invoke('update_save_dir', { saveDir: null });
      setSaveDir(null);
    } catch (e) {
      console.error(e);
    }
  };

  const handleAddFriend = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!peerIpInput.trim()) return;

    setAddingPeer(true);
    setAddMessage({ text: t('Friend Request Sent'), isError: false });

    try {
      let ip = peerIpInput.trim();
      let port = 52026;
      if (ip.includes(':')) {
        const parts = ip.split(':');
        ip = parts[0];
        const parsedPort = parseInt(parts[1], 10);
        if (!isNaN(parsedPort)) {
          port = parsedPort;
        }
      }

      // Connect to IP on target port
      const hostname = await invoke<string>('send_friend_request', {
        ip,
        port,
      });

      // Update alias if provided
      if (peerAliasInput.trim()) {
        await invoke('update_peer_alias', { ip, port, alias: peerAliasInput.trim() });
      }

      setAddMessage({ text: `${t('Friend Request Accepted')}: ${hostname}`, isError: false });
      setPeerIpInput('');
      setPeerAliasInput('');
      loadPeersAndConfig();
    } catch (e) {
      setAddMessage({ text: `${t('Failed to send request')}: ${String(e)}`, isError: true });
    } finally {
      setAddingPeer(false);
      setTimeout(() => setAddMessage(null), 5000);
    }
  };

  const handleRespondFriendRequest = async (accept: boolean) => {
    if (!pendingFriendRequest) return;
    try {
      await invoke('respond_friend_request', {
        requestId: pendingFriendRequest.request_id,
        accept,
      });
      if (accept) {
        loadPeersAndConfig();
      }
    } catch (e) {
      console.error(e);
    } finally {
      setPendingFriendRequest(null);
    }
  };

  const handleSelectFile = async () => {
    try {
      const file = await invoke<{ path: string; name: string; size: number } | null>('select_file_to_send');
      if (file) {
        setSelectedFile(file);
      }
    } catch (e) {
      console.error(e);
    }
  };

  const handleStartTransfer = async () => {
    if (!selectedFile || !targetPeerKey) return;

    // Find the peer info
    const [ip, portStr] = targetPeerKey.split(':');
    const port = parseInt(portStr, 10);
    const peer = trustedPeers.find(p => p.ip === ip && p.port === port);
    const peerDisplayName = peer ? (peer.alias || peer.hostname || targetPeerKey) : targetPeerKey;

    try {
      const transferId = await invoke<string>('send_file', {
        receiverIp: ip,
        receiverPort: port,
        filePath: selectedFile.path,
      });

      // Insert temporary transmission entry
      setTransmissions(prev => ({
        ...prev,
        [transferId]: {
          transferId,
          direction: 'send',
          filename: selectedFile.name,
          filesize: selectedFile.size,
          peerName: peerDisplayName,
          peerIp: ip,
          progress: 0,
          bytesTransferred: 0,
          speed: 0,
          status: 'transferring',
        }
      }));

      setSelectedFile(null);
    } catch (e) {
      showToast(t('Failed to send request') + `: ${e}`, true);
    }
  };

  const handleCancelTransfer = async (transferId: string) => {
    try {
      await invoke('cancel_transfer', { transferId });
      setTransmissions(prev => {
        const next = { ...prev };
        delete next[transferId];
        return next;
      });
    } catch (e) {
      console.error(e);
    }
  };

  const handleRemovePeer = async (ip: string, port: number) => {
    setConfirmConfig({
      title: t('Delete Friend'),
      message: t('Are you sure you want to remove this peer?'),
      onConfirm: async () => {
        try {
          await invoke('remove_trusted_peer', { ip, port });
          loadPeersAndConfig();
        } catch (e) {
          console.error(e);
        }
      }
    });
  };

  const handleSaveAlias = async (ip: string, port: number) => {
    const trimmed = editAliasInput.trim();
    try {
      await invoke('update_peer_alias', { ip, port, alias: trimmed });
      loadPeersAndConfig();
    } catch (e) {
      console.error('Failed to update peer alias:', e);
    } finally {
      setEditingPeerKey(null);
    }
  };

  const handleCopyIp = (ip: string) => {
    navigator.clipboard.writeText(ip);
    setCopiedIp(ip);
    setTimeout(() => setCopiedIp(null), 2000);
  };

  const handleOpenFile = async (path: string) => {
    try {
      await invoke('open_file', { path });
    } catch (e) {
      showToast(`${t('Failed to open file')}: ${e}`, true);
    }
  };

  const handleRevealInExplorer = async (path: string) => {
    try {
      await invoke('reveal_in_explorer', { path });
    } catch (e) {
      console.error(e);
    }
  };

  const handleDeleteFile = async (path: string, historyId: string) => {
    setConfirmConfig({
      title: t('Delete History'),
      message: t('Are you sure you want to delete this history record?'),
      onConfirm: async () => {
        try {
          // Read latest from localStorage first to prevent overwriting concurrent changes
          let currentHistory: HistoryRecord[] = [];
          try {
            const saved = localStorage.getItem('mtool_transfer_history');
            if (saved) {
              currentHistory = JSON.parse(saved) as HistoryRecord[];
            }
          } catch (e) {
            console.error(e);
          }

          const next = currentHistory.filter(r => r.id !== historyId);
          localStorage.setItem('mtool_transfer_history', JSON.stringify(next));
          setHistory(next);

          if (path) {
            await invoke('delete_local_file', { path });
          }
        } catch (e) {
          console.error(e);
        }
      }
    });
  };

  return (
    <div className="w-full space-y-6">
      {/* Title */}
      <div className="flex justify-between items-center mb-6 border-b th-border pb-4 shrink-0">
        <h2 className="th-text font-semibold text-lg flex items-center gap-2">
          <Network className="w-5 h-5 text-indigo-400" />
          {t('Lan File Transfer')}
        </h2>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
        {/* Left Column: Device Info & Friends */}
        <div className="lg:col-span-5 space-y-6">
          
          {/* My Device Card */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-xl relative overflow-hidden backdrop-blur-md">
            <div className="absolute top-3 right-3 flex items-center gap-1.5">
              <span className="relative flex h-2.5 w-2.5">
                <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"></span>
                <span className="relative inline-flex rounded-full h-2.5 w-2.5 bg-emerald-500"></span>
              </span>
              <span className="text-xs text-emerald-500 font-medium">Listening</span>
            </div>

            <h2 className="text-sm font-bold tracking-wider th-text-2 uppercase mb-4 flex items-center gap-2">
              <Sparkles className="w-4 h-4 text-indigo-400" />
              {t('My Device')}
            </h2>

            <div className="space-y-3.5">
              <div>
                <label className="text-xs th-text-muted block mb-1">{t('Local IPs')}</label>
                <div className="flex flex-wrap gap-1.5">
                  {localIps.map(ip => (
                    <button
                      key={ip}
                      onClick={() => handleCopyIp(ip)}
                      className="group flex items-center gap-1.5 px-2.5 py-1 rounded bg-indigo-500/10 border border-indigo-500/20 text-xs text-indigo-400 font-mono transition-all hover:bg-indigo-500/15"
                      title="Click to copy IP"
                    >
                      {ip}
                      {copiedIp === ip ? (
                        <Check className="w-3 h-3 text-emerald-400" />
                      ) : (
                        <Copy className="w-3 h-3 text-indigo-400 opacity-0 group-hover:opacity-100 transition-opacity" />
                      )}
                    </button>
                  ))}
                </div>
              </div>

              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="text-xs th-text-muted block mb-1">{t('Port')}</label>
                  <div className="text-sm font-mono th-text font-semibold">{localPort}</div>
                </div>
                <div>
                  <label className="text-xs th-text-muted block mb-1">Hostname</label>
                  <div className="text-sm th-text truncate font-semibold">{localHostname}</div>
                </div>
              </div>

              <div className="border-t th-border pt-3">
                <label className="text-xs th-text-muted block mb-1.5">{t('Save Path')}</label>
                <div className="flex items-center gap-2">
                  <div className="flex-1 text-xs font-mono th-text-2 bg-black/10 border th-border p-2 rounded-lg truncate select-text">
                    {saveDir || t('Default (Downloads)')}
                  </div>
                  <button
                    onClick={handleSelectSaveDir}
                    className="px-2.5 py-1.5 text-xs bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg transition-colors font-medium shrink-0"
                  >
                    {t('Change Path')}
                  </button>
                  {saveDir && (
                    <button
                      onClick={handleResetSaveDir}
                      className="p-1.5 text-xs text-rose-400 hover:bg-rose-500/10 rounded-lg border border-rose-500/20 transition-colors shrink-0"
                      title={t('Reset to Default')}
                    >
                      <XCircle className="w-4 h-4" />
                    </button>
                  )}
                </div>
              </div>
            </div>
          </div>

          {/* Add Friend Card */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-xl">
            <h2 className="text-sm font-bold tracking-wider th-text-2 uppercase mb-4 flex items-center gap-2">
              <UserPlus className="w-4 h-4 text-indigo-400" />
              {t('Add Peer')}
            </h2>

            <form onSubmit={handleAddFriend} className="space-y-3">
              <div>
                <input
                  type="text"
                  placeholder={t('Peer IP') + ' (e.g. 192.168.1.100)'}
                  value={peerIpInput}
                  onChange={(e) => setPeerIpInput(e.target.value)}
                  disabled={addingPeer}
                  className="w-full text-sm th-bg-input border th-border-subtle th-text rounded-lg px-3 py-2 focus:ring-2 focus:ring-indigo-500 focus:outline-none transition-all placeholder:text-slate-400 dark:placeholder:text-slate-500"
                  required
                />
              </div>
              <div>
                <input
                  type="text"
                  placeholder={t('Alias (Optional)')}
                  value={peerAliasInput}
                  onChange={(e) => setPeerAliasInput(e.target.value)}
                  disabled={addingPeer}
                  className="w-full text-sm th-bg-input border th-border-subtle th-text rounded-lg px-3 py-2 focus:ring-2 focus:ring-indigo-500 focus:outline-none transition-all placeholder:text-slate-400 dark:placeholder:text-slate-500"
                />
              </div>
              <button
                type="submit"
                disabled={addingPeer || !peerIpInput.trim()}
                className="w-full py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-lg text-sm font-medium transition-colors flex items-center justify-center gap-1.5 disabled:opacity-50 mt-1"
              >
                {addingPeer ? <RefreshCw className="w-4 h-4 animate-spin" /> : <UserPlus className="w-4 h-4" />}
                {t('Add Peer')}
              </button>

              {addMessage && (
                <div className={`p-2.5 rounded-lg text-xs font-medium animate-fade-in ${
                  addMessage.isError ? 'bg-rose-500/10 border border-rose-500/20 text-rose-400' : 'bg-indigo-500/10 border border-indigo-500/20 text-indigo-400'
                }`}>
                  {addMessage.text}
                </div>
              )}
            </form>
          </div>

          {/* Trusted Peers Card */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-xl flex flex-col min-h-[220px]">
            <h2 className="text-sm font-bold tracking-wider th-text-2 uppercase mb-3.5 flex items-center gap-2">
              <FileCheck className="w-4 h-4 text-indigo-400" />
              {t('Trusted Peers')}
            </h2>

            <div className="flex-1 overflow-y-auto space-y-2 pr-1 max-h-[300px]">
              {trustedPeers.length === 0 ? (
                <div className="h-full flex flex-col items-center justify-center text-xs th-text-muted py-8 text-center">
                  <Network className="w-8 h-8 mb-2 opacity-30" />
                  {t('No trusted peers yet. Add a peer by IP above!')}
                </div>
              ) : (
                trustedPeers.map(peer => (
                  <div key={`${peer.ip}:${peer.port}`} className="flex items-center justify-between p-3 rounded-lg border th-border th-bg-surface-h transition-all group">
                    <div className="overflow-hidden mr-2">
                      <div className="flex items-center gap-1.5 max-w-full">
                        {editingPeerKey === `${peer.ip}:${peer.port}` ? (
                          <input
                            type="text"
                            value={editAliasInput}
                            onChange={(e) => setEditAliasInput(e.target.value)}
                            onKeyDown={(e) => {
                              if (e.key === 'Enter') handleSaveAlias(peer.ip, peer.port);
                              if (e.key === 'Escape') setEditingPeerKey(null);
                            }}
                            onBlur={() => handleSaveAlias(peer.ip, peer.port)}
                            className="text-xs px-1.5 py-0.5 rounded th-bg-input border th-border th-text outline-none focus:ring-1 focus:ring-indigo-500 w-32 font-medium"
                            placeholder={peer.hostname || 'Alias'}
                            autoFocus
                          />
                        ) : (
                          <div className="flex items-center gap-1.5 group/alias min-w-0 max-w-full">
                            <span className="font-semibold text-xs th-text truncate max-w-[150px]" title={peer.alias || peer.hostname || 'Device'}>
                              {peer.alias || peer.hostname || 'Device'}
                            </span>
                            {peer.alias && (
                              <span className="text-[10px] th-text-muted font-mono truncate max-w-[100px]" title={peer.hostname}>
                                ({peer.hostname})
                              </span>
                            )}
                            <button
                              onClick={() => {
                                setEditingPeerKey(`${peer.ip}:${peer.port}`);
                                setEditAliasInput(peer.alias || '');
                              }}
                              className="opacity-0 group-hover/alias:opacity-100 p-0.5 text-indigo-400 hover:text-indigo-300 transition-opacity focus:opacity-100"
                              title={t('Edit Alias')}
                            >
                              <Edit3 className="w-3 h-3" />
                            </button>
                          </div>
                        )}
                      </div>
                      <span className="text-[10px] font-mono th-text-muted block mt-0.5">
                        {peer.ip}{peer.port !== 52026 ? `:${peer.port}` : ''}
                      </span>
                    </div>

                    <div className="flex items-center gap-1">
                      <button
                        onClick={() => {
                          setTargetPeerKey(`${peer.ip}:${peer.port}`);
                          handleSelectFile();
                        }}
                        className="px-2 py-1 text-[11px] bg-indigo-500/15 text-indigo-400 rounded-md font-medium hover:bg-indigo-500/25 transition-colors flex items-center gap-1"
                      >
                        <Send className="w-3 h-3" />
                        {t('Send File')}
                      </button>
                      <button
                        onClick={() => handleRemovePeer(peer.ip, peer.port)}
                        className="p-1.5 text-rose-400 hover:bg-rose-500/10 rounded-md transition-colors"
                        title={t('Delete')}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>

        </div>

        {/* Right Column: Send File, Active Progress & History */}
        <div className="lg:col-span-7 space-y-6">
          
          {/* Send File Card */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-xl">
            <h2 className="text-sm font-bold tracking-wider th-text-2 uppercase mb-4 flex items-center gap-2">
              <Send className="w-4 h-4 text-indigo-400" />
              {t('Send File')}
            </h2>

            <div className="space-y-4">
              {/* File selection dropzone */}
              <div
                onClick={handleSelectFile}
                className="border-2 border-dashed th-border hover:border-indigo-500 bg-black/5 hover:bg-indigo-500/5 transition-all rounded-xl p-6 flex flex-col items-center justify-center cursor-pointer text-center group"
              >
                <Send className="w-10 h-10 mb-3 th-text-muted group-hover:text-indigo-400 transition-colors" />
                <span className="text-xs th-text-2 font-medium mb-1">{t('Select File')}</span>
                <span className="text-[10px] th-text-muted">{t('Drag and drop a file here, or click to browse.')}</span>
              </div>

              {selectedFile && (
                <div className="p-3 rounded-lg bg-indigo-500/5 border border-indigo-500/10 space-y-3 animate-fade-in">
                  <div className="flex items-start justify-between">
                    <div className="overflow-hidden">
                      <label className="text-[10px] th-text-muted block">{t('Selected File')}</label>
                      <span className="text-sm font-medium th-text truncate block">{selectedFile.name}</span>
                      <span className="text-xs th-text-muted font-mono">{formatBytes(selectedFile.size)}</span>
                    </div>
                    <button
                      onClick={() => setSelectedFile(null)}
                      className="text-rose-400 hover:bg-rose-500/10 p-1 rounded-md transition-colors"
                    >
                      <XCircle className="w-4 h-4" />
                    </button>
                  </div>

                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3 pt-2 border-t th-border">
                    <div>
                      <label className="text-[10px] th-text-muted block mb-1">{t('Target Peer')}</label>
                      <CustomSelect 
                        value={targetPeerKey}
                        onChange={(val: string) => setTargetPeerKey(val)}
                        options={[
                          { value: '', label: t('Choose a peer...') },
                          ...trustedPeers.map(p => ({
                            value: `${p.ip}:${p.port}`,
                            label: `${p.alias || p.hostname || p.ip} (${p.ip}:${p.port})`
                          }))
                        ]}
                        className="w-full px-3 py-2 border th-border-subtle th-text rounded-lg text-xs focus:ring-1 focus:ring-indigo-500 transition-all"
                        menuClassName="w-full left-0 min-w-[8rem]"
                      />
                    </div>

                    <div className="flex items-end">
                      <button
                        onClick={handleStartTransfer}
                        disabled={!targetPeerKey}
                        className="w-full py-2 bg-indigo-600 hover:bg-indigo-700 disabled:bg-indigo-800 disabled:opacity-50 text-white rounded-lg text-xs font-semibold transition-colors flex items-center justify-center gap-1.5"
                      >
                        <Send className="w-3.5 h-3.5" />
                        {t('Start Transfer')}
                      </button>
                    </div>
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Ongoing Transmissions Card */}
          {Object.keys(transmissions).length > 0 && (
            <div className="th-bg-card border th-border rounded-xl p-5 shadow-xl space-y-4">
              <h2 className="text-sm font-bold tracking-wider th-text-2 uppercase flex items-center gap-2">
                <RefreshCw className="w-4 h-4 text-indigo-400 animate-spin" />
                {t('Transmissions')}
              </h2>

              <div className="space-y-3.5 max-h-[300px] overflow-y-auto pr-1">
                {Object.values(transmissions).map(tTask => (
                  <div key={tTask.transferId} className="border th-border rounded-lg p-3.5 th-bg-surface-h space-y-2.5 relative overflow-hidden">
                    {/* Background glow for progress */}
                    <div 
                      className="absolute inset-y-0 left-0 bg-indigo-500/5 transition-all duration-300"
                      style={{ width: `${tTask.progress}%` }}
                    />
                    
                    <div className="flex items-start justify-between relative z-10">
                      <div className="overflow-hidden">
                        <span className="flex items-center gap-1.5 text-xs font-bold uppercase mb-0.5">
                          {tTask.direction === 'send' ? (
                            <span className="text-indigo-400 flex items-center gap-1">
                              <Send className="w-3.5 h-3.5" /> Sending to
                            </span>
                          ) : (
                            <span className="text-emerald-400 flex items-center gap-1">
                              <Download className="w-3.5 h-3.5" /> Receiving from
                            </span>
                          )}
                          <span className="th-text truncate">{tTask.peerName || 'Device'}</span>
                        </span>
                        <span className="text-sm th-text font-semibold truncate block max-w-md">{tTask.filename}</span>
                      </div>

                      <button
                        onClick={() => handleCancelTransfer(tTask.transferId)}
                        className="text-rose-400 hover:bg-rose-500/10 p-1.5 rounded-md transition-colors"
                        title={t('Cancel')}
                      >
                        <XCircle className="w-4.5 h-4.5" />
                      </button>
                    </div>

                    <div className="space-y-1 relative z-10">
                      {/* Progress bar container */}
                      <div className="w-full bg-slate-800 rounded-full h-2 overflow-hidden border border-slate-700/50">
                        <div
                          className="bg-indigo-500 h-full rounded-full transition-all duration-300 relative"
                          style={{ width: `${tTask.progress}%` }}
                        >
                          <div className="absolute inset-0 bg-white/25 animate-pulse" />
                        </div>
                      </div>

                      <div className="flex items-center justify-between text-[11px] th-text-muted font-semibold">
                        <span>{formatBytes(tTask.bytesTransferred)} / {formatBytes(tTask.filesize)}</span>
                        <span>{tTask.progress}%</span>
                        <span>{formatSpeed(tTask.speed)}</span>
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Transfer History Card */}
          <div className="th-bg-card border th-border rounded-xl p-5 shadow-xl flex flex-col min-h-[300px]">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-sm font-bold tracking-wider th-text-2 uppercase flex items-center gap-2">
                <History className="w-4 h-4 text-indigo-400" />
                {t('History')}
              </h2>
              {history.length > 0 && (
                <button
                  onClick={() => {
                    setConfirmConfig({
                      title: t('Clear History'),
                      message: t('Are you sure you want to clear all history records?'),
                      onConfirm: () => {
                        setHistory([]);
                        localStorage.removeItem('mtool_transfer_history');
                      }
                    });
                  }}
                  className="text-xs th-text-muted hover:th-text transition-colors font-medium"
                >
                  {t('Clear')}
                </button>
              )}
            </div>

            <div className="flex-1 overflow-y-auto space-y-2 pr-1 max-h-[400px]">
              {history.length === 0 ? (
                <div className="h-full flex flex-col items-center justify-center text-xs th-text-muted py-12 text-center">
                  <History className="w-8 h-8 mb-2 opacity-30" />
                  {t('No history records.')}
                </div>
              ) : (
                history.map(record => (
                  <div key={record.id} className="flex items-center justify-between p-3.5 rounded-lg border th-border th-bg-surface-h transition-all group">
                    <div className="overflow-hidden mr-3">
                      <div className="flex items-center gap-1.5 flex-wrap">
                        {record.direction === 'send' ? (
                          <span className="text-[10px] font-bold text-indigo-400 uppercase tracking-tight flex items-center gap-0.5 shrink-0">
                            <Send className="w-2.5 h-2.5" /> Sent
                          </span>
                        ) : (
                          <span className="text-[10px] font-bold text-emerald-400 uppercase tracking-tight flex items-center gap-0.5 shrink-0">
                            <Download className="w-2.5 h-2.5" /> Recv
                          </span>
                        )}
                        <span className="text-xs font-semibold th-text truncate max-w-xs" title={record.filename}>
                          {record.filename}
                        </span>
                      </div>
                      
                      <div className="flex items-center gap-2 mt-1 text-[10px] th-text-muted">
                        <span>{formatBytes(record.filesize)}</span>
                        <span>•</span>
                        <span>{record.peerName}</span>
                        <span>•</span>
                        <span>{new Date(record.timestamp).toLocaleString()}</span>
                      </div>
                    </div>

                    <div className="flex items-center gap-1.5 shrink-0">
                      {record.status === 'success' ? (
                        <>
                          {record.savePath && (
                            <>
                              <button
                                onClick={() => handleOpenFile(record.savePath!)}
                                className="p-1.5 text-indigo-400 hover:bg-indigo-500/10 rounded-md transition-colors"
                                title={t('Open File')}
                              >
                                <Eye className="w-4 h-4" />
                              </button>
                              <button
                                onClick={() => handleRevealInExplorer(record.savePath!)}
                                className="p-1.5 text-indigo-400 hover:bg-indigo-500/10 rounded-md transition-colors"
                                title={t('Show in Finder')}
                              >
                                <FolderOpen className="w-4 h-4" />
                              </button>
                            </>
                          )}
                          <span className="text-[10px] px-2 py-0.5 rounded bg-emerald-500/10 border border-emerald-500/25 text-emerald-400 font-semibold uppercase tracking-tight">
                            Success
                          </span>
                        </>
                      ) : (
                        <span className="text-[10px] px-2 py-0.5 rounded bg-rose-500/10 border border-rose-500/25 text-rose-400 font-semibold uppercase tracking-tight">
                          {record.status === 'rejected' ? 'Rejected' : 'Failed'}
                        </span>
                      )}
                      
                      <button
                        onClick={() => handleDeleteFile(record.savePath || '', record.id)}
                        className="p-1.5 text-rose-400 hover:bg-rose-500/10 rounded-md transition-colors"
                        title={t('Delete')}
                      >
                        <Trash2 className="w-3.5 h-3.5" />
                      </button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>

        </div>
      </div>

      {/* Overlay Modal: Friend Request Handshake */}
      {pendingFriendRequest && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-md flex items-center justify-center z-50 animate-fade-in">
          <div className="th-bg-card border th-border rounded-2xl p-6 shadow-2xl max-w-sm w-full mx-4 border-indigo-500/30 overflow-hidden relative">
            {/* Ambient light glow in card */}
            <div className="absolute -top-12 -left-12 w-24 h-24 bg-indigo-500/20 rounded-full blur-xl pointer-events-none" />

            <div className="flex flex-col items-center text-center space-y-4">
              <div className="w-12 h-12 bg-indigo-500/10 rounded-full flex items-center justify-center text-indigo-400 border border-indigo-500/25">
                <Network className="w-6 h-6 animate-pulse" />
              </div>

              <div className="space-y-2">
                <h3 className="text-lg font-bold th-text">{t('Incoming Friend Request')}</h3>
                <p className="text-xs th-text-muted font-mono leading-relaxed">
                  <span className="font-semibold th-text block text-sm mb-1">{pendingFriendRequest.sender_name}</span>
                  ({pendingFriendRequest.sender_ip})
                </p>
                <p className="text-xs th-text-2">{t('wants to add you as a peer. Agree?')}</p>
              </div>

              <div className="flex gap-3 w-full pt-2">
                <button
                  onClick={() => handleRespondFriendRequest(false)}
                  className="flex-1 py-2 text-xs border th-border-subtle th-text-2 hover:bg-rose-500/15 hover:text-rose-400 rounded-xl transition-all font-semibold"
                >
                  {t('Reject')}
                </button>
                <button
                  onClick={() => handleRespondFriendRequest(true)}
                  className="flex-1 py-2 text-xs bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl shadow-lg shadow-indigo-500/10 hover:shadow-indigo-500/20 transition-all font-semibold"
                >
                  {t('Accept')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Custom Confirmation Modal */}
      {confirmConfig && (
        <div className="fixed inset-0 bg-black/60 backdrop-blur-md flex items-center justify-center z-50 animate-fade-in">
          <div className="th-bg-card border th-border rounded-2xl p-6 shadow-2xl max-w-sm w-full mx-4 border-indigo-500/30 overflow-hidden relative">
            <div className="flex flex-col items-center text-center space-y-4">
              <div className="w-12 h-12 bg-rose-500/10 rounded-full flex items-center justify-center text-rose-400 border border-rose-500/25 animate-pulse">
                <Trash2 className="w-6 h-6" />
              </div>

              <div className="space-y-2">
                <h3 className="text-lg font-bold th-text">{confirmConfig.title}</h3>
                <p className="text-xs th-text-muted leading-relaxed">
                  {confirmConfig.message}
                </p>
              </div>

              <div className="flex gap-3 w-full pt-2">
                <button
                  onClick={() => {
                    if (confirmConfig.onCancel) confirmConfig.onCancel();
                    setConfirmConfig(null);
                  }}
                  className="flex-1 py-2 text-xs border th-border-subtle th-text-2 hover:bg-slate-500/15 rounded-xl transition-all font-semibold"
                >
                  {t('Cancel')}
                </button>
                <button
                  onClick={() => {
                    confirmConfig.onConfirm();
                    setConfirmConfig(null);
                  }}
                  className="flex-1 py-2 text-xs bg-rose-600 hover:bg-rose-700 text-white rounded-xl shadow-lg shadow-rose-500/10 hover:shadow-rose-500/20 transition-all font-semibold"
                >
                  {t('Confirm')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Floating Toast Notification */}
      {toastMessage && (
        <div className="fixed bottom-5 right-5 z-50 animate-fade-in">
          <div className={`px-4 py-3 rounded-xl shadow-2xl border backdrop-blur-md text-xs font-semibold flex items-center gap-2 ${
            toastMessage.isError 
              ? 'bg-rose-500/10 border-rose-500/25 text-rose-400' 
              : 'bg-indigo-500/10 border-indigo-500/25 text-indigo-400'
          }`}>
            <span className={`w-2 h-2 rounded-full ${toastMessage.isError ? 'bg-rose-500 animate-pulse' : 'bg-indigo-500 animate-pulse'}`} />
            {toastMessage.text}
          </div>
        </div>
      )}
    </div>
  );
}
