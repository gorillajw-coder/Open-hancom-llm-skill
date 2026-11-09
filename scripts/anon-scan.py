#!/usr/bin/env python3 -I
"""Residual scan of anonymized HWPX files: every ZIP member, raw + utf-8 + utf-16 views.
usage: anon-scan.py TERMS_FILE OUT_JSON FILE.hwpx...
TERMS_FILE (original entity strings, one per line) stays outside git; output reports counts only,
never the terms themselves."""
import sys, zipfile, json, re, hashlib, zlib
terms=[t for t in open(sys.argv[1],encoding="utf8").read().split("\n") if t.strip()]
pats={"email_not_example":re.compile(r"[\w.+-]+@(?!example\.com)[\w-]+\.[\w.]+"),
      "phone_like":re.compile(r"(?<![\w.])\+?\d{2,4}[ -]\d{2,4}[ -]\d{3,4}(?:[ -]\d{2,4})?(?![\w.])"),
      "rrn_like":re.compile(r"\d{6}-\d{7}")}
def views(b):
    v=[b]
    for enc in ("utf-8","utf-16-le","utf-16-be"):
        try: v.append(b.decode(enc,"ignore").encode("utf-8"))
        except Exception: pass
    return v
def png_blank(b):
    if not b.startswith(b"\x89PNG"): return None
    idat=b""; i=8
    while i<len(b):
        n=int.from_bytes(b[i:i+4],"big"); t=b[i+4:i+8]
        if t==b"IDAT": idat+=b[i+8:i+8+n]
        i+=12+n
    raw=zlib.decompress(idat); return len(set(raw))<=3
res={"files":[],"total_hits":0}
for f in sys.argv[3:]:
    z=zipfile.ZipFile(f); ent={"file":f.split("/")[-1],"members":[]}
    for info in z.infolist():
        d=z.read(info.filename); hits=0; pat={}
        for t in terms:
            tl=t.lower().encode()
            hits+=sum(1 for v in views(d) if tl in v.lower())
            hits+=1 if t.lower().encode("utf-16-le") in d.lower() else 0
        txt=d.decode("utf-8","ignore")+d.decode("utf-16-le","ignore")
        for k,p in pats.items():
            n=len([x for x in p.findall(txt) if x!='000-0000-0000']); pat[k]=n; hits+=n
        m={"name":info.filename,"size":len(d),"sha256":hashlib.sha256(d).hexdigest()[:16],"hits":hits,"pattern_hits":pat}
        if info.filename.lower().endswith(".png"): m["blank_image"]=png_blank(d)
        ent["members"].append(m); res["total_hits"]+=hits
    res["files"].append(ent)
json.dump(res,open(sys.argv[2],"w"),ensure_ascii=False,indent=1)
print("total_hits",res["total_hits"],[ (e["file"],sum(m["hits"] for m in e["members"])) for e in res["files"]])
for e in res["files"]:
    for m in e["members"]:
        if m["hits"] or m.get("blank_image") is not None: print(e["file"],m["name"],m["hits"],m.get("blank_image"))
