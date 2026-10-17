# Klien baris perintah (`udm-cli`)

`udm-cli` menjalankan perintah yang sama dengan aplikasi. Secara bawaan ia menjalankan mesinnya sendiri pada folder data; dengan `--control PORT` ia terhubung ke UDM yang sedang berjalan.

```powershell
udm-cli add http://127.0.0.1:8099/files/big.bin --dir .udm-dev\out --connections 8 --wait
udm-cli verify .udm-dev\out\big.bin --sha256 <hex>
udm-cli list
udm-cli exec pause d-000001
udm-cli run --script .udm-dev\script.jsonl
udm-cli --control 7878 list
```

| Opsi | Arti |
|---|---|
| `--data-dir DIR` | Pengaturan, antrean, dan status lanjutan. Bawaan: folder data sistem operasi. |
| `--control PORT` | Kirim permintaan ke UDM yang berjalan di `127.0.0.1:PORT`. |
| `--locale id` / `--locale en` | Bahasa pesan galat (bawaan `id`). |

- Hasil dicetak sebagai satu nilai JSON per baris. Galat ditulis ke stderr sebagai kalimat terlokalisasi diikuti kunci pesannya, dan kode keluar 1.
- `run --script` membaca satu permintaan kontrol per baris (JSON yang sama dengan port kontrol). Baris kosong dan baris berawalan `#` dilewati. Ukuran skrip maksimal 8 MiB.
- Jika aplikasi UDM sudah berjalan pada folder data yang sama, `udm-cli` menolak menjalankan mesin kedua (`error-cli-app-running`); gunakan `--control`.
- `add --wait` menunggu sampai unduhan selesai (keluar 0) atau gagal (keluar 1). Penantian yang dihentikan membiarkan unduhan tetap di antrean; unduhan dilanjutkan dengan aman nanti.
