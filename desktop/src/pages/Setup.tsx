import { useEffect, useState } from "react";
import { doctor, listEncoders, listModels, type Check, type EncoderInfo, type ModelInfo } from "../api";

export default function SetupPage() {
  const [checks, setChecks] = useState<Check[] | null>(null);
  const [encoders, setEncoders] = useState<EncoderInfo[] | null>(null);
  const [models, setModels] = useState<ModelInfo[]>([]);

  useEffect(() => {
    doctor().then(setChecks);
    listEncoders().then(setEncoders);
    listModels().then(setModels);
  }, []);

  return (
    <div className="page">
      <h1>Setup</h1>
      <p className="lede">mx drives ffmpeg and yt-dlp. Here's what it found on this machine.</p>

      <div className="checks">
        {checks === null && (
          <div className="row muted">
            <div className="spinner" /> Checking...
          </div>
        )}
        {checks?.map((c) => (
          <div className="check" key={c.name}>
            <span className={c.found ? "ok" : "missing"}>{c.found ? "✓" : "!"}</span>
            <strong>{c.name}</strong>
            <span className={c.found ? "muted ellipsis" : ""} title={c.found ?? undefined}>
              {c.found ?? c.fix}
            </span>
          </div>
        ))}
      </div>

      <h3 className="section">Video encoders that work here</h3>
      {encoders === null ? (
        <div className="row muted">
          <div className="spinner" /> Test-encoding...
        </div>
      ) : (
        <div className="checks">
          {(["h264", "hevc", "av1"] as const).map((codec) => {
            const list = encoders.filter((e) => e.codec === codec);
            return (
              <div className="check" key={codec}>
                <span className={list.length ? "ok" : "missing"}>{list.length ? "✓" : "–"}</span>
                <strong>{codec.toUpperCase()}</strong>
                <span className="muted">
                  {list.length ? list.map((e) => `${e.name}${e.hardware ? " (GPU)" : ""}`).join(", ") : "none"}
                </span>
              </div>
            );
          })}
        </div>
      )}
      <p className="faint" style={{ maxWidth: 760 }}>
        The GPU encodes first when it's faster. Below 720p the CPU wins, so small outputs use it.
      </p>

      <h3 className="section">Speech models</h3>
      <div className="checks">
        {models.map((m) => (
          <div className="check" key={m.name}>
            <span className={m.installed ? "ok" : "faint"}>{m.installed ? "✓" : "·"}</span>
            <strong>{m.name}</strong>
            <span className="muted">
              {m.mb} MB · {m.englishOnly ? "English only" : "multilingual"}
              {m.installed ? " · installed" : " · downloads when first used"}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
