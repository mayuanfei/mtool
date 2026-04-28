import { Copy, Download, Clipboard, Trash2, QrCode } from 'lucide-react';
import { useState } from 'react';

export function TextToQr() {
  const [payload, setPayload] = useState('https://mtool.app/qrcode');
  const [redundancy, setRedundancy] = useState('Q');
  const [resolution, setResolution] = useState(512);

  return (
    <div className="w-full h-full flex flex-col">
      <div className="mb-6 border-b border-slate-800 pb-4">
        <h2 className="text-white font-semibold text-lg flex items-center gap-2 mb-1">
          <span className="text-indigo-400 px-1"><QrCode className="w-5 h-5" /></span> Text to QR
        </h2>
        <span className="px-2 py-0.5 rounded bg-slate-800 text-slate-400 text-[10px] font-mono border border-slate-700 w-max inline-block">v1.1.0</span>
      </div>

      <div className="flex-1 flex flex-col lg:flex-row gap-6 min-h-0">
        
        {/* Left Column: Config */}
        <div className="flex-[3] flex flex-col gap-6 overflow-y-auto pr-2">
          
          {/* Raw Payload */}
          <div className="bg-slate-900 border border-slate-800 rounded-xl p-5 shadow-2xl">
            <div className="flex justify-between items-center mb-3">
              <h3 className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter flex items-center gap-2">
                Raw Payload
              </h3>
              <span className="text-xs text-slate-500 font-mono bg-slate-950 px-2 py-0.5 rounded border border-slate-800">{payload.length} / 2048 chars</span>
            </div>
            
            <textarea
              value={payload}
              onChange={(e) => setPayload(e.target.value)}
              className="w-full bg-slate-950/50 border border-slate-700/50 rounded-md p-4 text-slate-300 text-sm focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-none h-32 mb-3 shadow-inner"
              placeholder="Enter URL, text, or JSON payload..."
            />
            
            <div className="flex justify-end gap-2">
               <button className="p-2 text-slate-500 hover:text-white hover:bg-slate-800 rounded transition-colors border border-transparent hover:border-slate-700" title="Paste from clipboard">
                  <Clipboard className="w-4 h-4" />
               </button>
               <button className="p-2 text-slate-500 hover:text-red-400 hover:bg-slate-800 rounded transition-colors border border-transparent hover:border-slate-700" title="Clear payload" onClick={() => setPayload('')}>
                  <Trash2 className="w-4 h-4" />
               </button>
            </div>
          </div>

          <div className="flex gap-6">
            {/* Redundancy */}
            <div className="flex-1 bg-slate-900 border border-slate-800 rounded-xl p-5 shadow-2xl">
              <h3 className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter flex items-center gap-2 mb-4">
                Redundancy Level
              </h3>
              <div className="grid grid-cols-4 gap-2 bg-slate-950/50 p-1.5 rounded-lg border border-slate-800/50">
                {['L', 'M', 'Q', 'H'].map((level) => (
                  <button
                    key={level}
                    onClick={() => setRedundancy(level)}
                    className={`text-xs py-2 rounded font-medium transition-colors ${
                      redundancy === level 
                        ? 'bg-slate-800 text-indigo-400 border border-indigo-500/30 shadow-sm' 
                        : 'text-slate-500 hover:text-slate-300 border border-transparent'
                    }`}
                  >
                    {level} <span className="opacity-50 text-[10px]">({
                      level === 'L' ? '7%' : level === 'M' ? '15%' : level === 'Q' ? '25%' : '30%'
                    })</span>
                  </button>
                ))}
              </div>
            </div>

            {/* Matrix Resolution */}
            <div className="flex-1 bg-slate-900 border border-slate-800 rounded-xl p-5 shadow-2xl">
              <h3 className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter flex items-center gap-2 mb-6">
                Matrix Resolution
              </h3>
              
              <div className="px-2">
                 <input 
                    type="range" min="256" max="2048" step="256"
                    value={resolution}
                    onChange={(e) => setResolution(parseInt(e.target.value))}
                    className="w-full h-1.5 bg-slate-800 rounded-lg appearance-none cursor-pointer accent-indigo-500"
                 />
                 <div className="flex justify-between text-[10px] text-slate-500 mt-3 font-mono">
                    <span>256px</span>
                    <span className="text-indigo-400">512px</span>
                    <span>1024px</span>
                    <span>2048px</span>
                 </div>
              </div>
            </div>
          </div>
          
          {/* Chromatic Injection */}
          <div className="bg-slate-900 border border-slate-800 rounded-xl p-5 shadow-2xl">
             <h3 className="text-[11px] font-bold text-slate-400 uppercase tracking-tighter flex items-center gap-2 mb-4">
                Chromatic Injection
             </h3>
             <div className="flex items-center gap-4">
                <div className="flex gap-2">
                   <button className="w-8 h-8 rounded-full bg-indigo-500 ring-2 ring-offset-2 ring-offset-slate-900 ring-indigo-500 cursor-pointer"></button>
                   <button className="w-8 h-8 rounded-full bg-emerald-400 cursor-pointer hover:ring-2 ring-offset-2 ring-offset-slate-900 ring-slate-600 transition-all"></button>
                   <button className="w-8 h-8 rounded-full bg-white cursor-pointer hover:ring-2 ring-offset-2 ring-offset-slate-900 ring-slate-600 transition-all"></button>
                </div>
                <div className="h-8 w-px bg-slate-800"></div>
                <div className="flex-1 flex items-center bg-slate-950/50 border border-slate-700/50 rounded-md px-3 py-1.5 focus-within:border-indigo-500 transition-colors shadow-inner">
                   <span className="text-slate-500 text-xs font-mono mr-2">HEX</span>
                   <input type="text" value="#6366f1" readOnly className="bg-transparent border-none text-slate-300 text-sm font-mono focus:outline-none w-full" />
                </div>
             </div>
          </div>

        </div>

        {/* Right Column: Preview */}
        <div className="flex-[2] flex flex-col gap-4">
          <div className="flex-1 bg-slate-900 border border-slate-800 rounded-xl flex flex-col justify-center items-center relative overflow-hidden shadow-2xl">
             
             {/* Corner brackets */}
             <div className="absolute top-4 left-4 w-6 h-6 border-t border-l border-slate-700/50"></div>
             <div className="absolute top-4 right-4 w-6 h-6 border-t border-r border-slate-700/50"></div>
             <div className="absolute bottom-4 left-4 w-6 h-6 border-b border-l border-slate-700/50"></div>
             <div className="absolute bottom-4 right-4 w-6 h-6 border-b border-r border-slate-700/50"></div>
             
             <div className="absolute top-6 left-0 right-0 flex justify-center">
                <span className="text-[10px] font-bold text-indigo-400 uppercase flex items-center gap-2 bg-slate-900 px-2 tracking-widest">
                   <span className="w-1.5 h-1.5 rounded-full bg-indigo-500 animate-pulse"></span> Preview
                </span>
             </div>

             {/* Mock QR Code Body */}
             <div className="relative mt-8 group">
                <div className="absolute -inset-4 bg-indigo-500/20 blur-2xl rounded-full opacity-0 group-hover:opacity-100 transition-opacity duration-1000"></div>
                <div className="relative w-64 h-64 bg-slate-950 border border-slate-800 rounded-xl p-4 flex flex-col justify-between shadow-xl">
                   
                   {/* QR Corner Squares mock */}
                   <div className="flex justify-between w-full h-12">
                      <div className="w-12 h-12 border-[4px] border-indigo-500 rounded-sm p-1">
                         <div className="w-full h-full bg-indigo-500 rounded-sm"></div>
                      </div>
                      <div className="w-12 h-12 border-[4px] border-indigo-500 rounded-sm p-1">
                         <div className="w-full h-full bg-indigo-500 rounded-sm"></div>
                      </div>
                   </div>

                   {/* Mock Data dots */}
                   <div className="flex-1 my-2 flex flex-col gap-1.5 justify-center opacity-80 mix-blend-screen">
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px]"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px] ml-1"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px]"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px] ml-1"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px]"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px] ml-1"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px]"></div>
                      <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px] ml-1"></div>
                   </div>

                   <div className="flex justify-between w-full h-12">
                      <div className="w-12 h-12 border-[4px] border-indigo-500 rounded-sm p-1">
                         <div className="w-full h-full bg-indigo-500 rounded-sm"></div>
                      </div>
                      <div className="flex-1 ml-4 h-full flex flex-col gap-1.5 justify-end mix-blend-screen">
                         <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px]"></div>
                         <div className="w-full h-1.5 bg-[radial-gradient(circle,#6366f1_50%,transparent_50%)] bg-[length:6px_6px]"></div>
                      </div>
                   </div>

                </div>
             </div>
             
             <div className="mt-8 text-center space-y-1 text-xs text-slate-500 font-mono">
                <p>Format: PNG</p>
                <p>Dimensions: {resolution}x{resolution}px</p>
             </div>
          </div>
          
          <div className="flex gap-4">
            <button className="flex-1 flex items-center justify-center gap-2 py-2.5 bg-slate-800 text-slate-300 rounded-md hover:bg-slate-700 transition-colors border border-slate-700 text-sm font-medium shadow-sm focus:outline-none">
              <Copy className="w-4 h-4" /> Copy Image
            </button>
            <button className="flex-1 flex items-center justify-center gap-2 py-2.5 bg-indigo-600 text-white rounded-md hover:bg-indigo-500 transition-colors text-sm font-medium shadow-lg shadow-indigo-600/20 focus:outline-none">
              <Download className="w-4 h-4" /> Download
            </button>
          </div>
        </div>

      </div>
    </div>
  );
}
