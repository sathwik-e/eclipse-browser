#!/usr/bin/env python3
"""Build a valid macOS UDZO (zlib-compressed UDIF) .dmg containing an HFS+ volume
from a staging directory when hdiutil/DiskArbitration is unavailable in sandboxed environments.
"""

import base64
import math
import os
import plistlib
import stat
import struct
import sys
import time
import zlib
from pathlib import Path

BLOCK_SIZE = 4096
SECTOR_SIZE = 512
HFS_EPOCH_OFFSET = 2082844800  # Seconds between 1904-01-01 and 1970-01-01


def to_hfs_time(unix_ts: float) -> int:
    return int(unix_ts) + HFS_EPOCH_OFFSET


def encode_hfs_uni_str(name: str) -> bytes:
    u16 = name.encode("utf-16-be")
    char_count = len(u16) // 2
    return struct.pack(">H", char_count) + u16


def hfs_fast_unicode_compare_key(name: str) -> tuple:
    # Case-insensitive comparison key approximation for HFS+ (kHFSCaseFold)
    return tuple(ord(c) for c in name.lower())


def make_catalog_key(parent_cnid: int, name: str) -> bytes:
    uni = encode_hfs_uni_str(name)
    key_len = 4 + len(uni)  # parentID (4) + HFSUniStr255
    return struct.pack(">HI", key_len, parent_cnid) + uni


def pack_fork_data(logical_size: int, start_block: int, block_count: int) -> bytes:
    # HFSPlusForkData: logicalSize(Q), clumpSize(I), totalBlocks(I), extents(8 * (startBlock(I), blockCount(I)))
    extents = bytearray(64)
    if block_count > 0:
        struct.pack_into(">II", extents, 0, start_block, block_count)
    clump_size = max(block_count * BLOCK_SIZE, BLOCK_SIZE) if block_count > 0 else 0
    return struct.pack(">QII", logical_size, clump_size, block_count) + bytes(extents)


def build_btree(node_size: int, total_nodes: int, max_key_len: int, key_compare_type: int, records: list[bytes]) -> bytes:
    """Build a single-level (or multi-leaf) HFS+ B-tree file with a header node and leaf nodes."""
    # Pack records into leaf nodes
    leaf_nodes = []
    if records:
        cur_recs = []
        cur_bytes = 14 + 2  # BTNodeDescriptor (14) + free space offset (2)
        for rec in records:
            rec_aligned = rec + (b"\x00" if len(rec) % 2 else b"")
            if cur_recs and cur_bytes + len(rec_aligned) + 2 > node_size:
                leaf_nodes.append(cur_recs)
                cur_recs = [rec_aligned]
                cur_bytes = 14 + 2 + len(rec_aligned) + 2
            else:
                cur_recs.append(rec_aligned)
                cur_bytes += len(rec_aligned) + 2
        if cur_recs:
            leaf_nodes.append(cur_recs)

    # Ensure total_nodes is large enough for header + leaves (+ index if >1 leaf)
    need_index = len(leaf_nodes) > 1
    used_nodes = 1 + (1 if need_index else 0) + len(leaf_nodes)
    if total_nodes < used_nodes:
        total_nodes = used_nodes + 8

    buf = bytearray(total_nodes * node_size)

    if len(leaf_nodes) == 0:
        tree_depth = 0
        root_node = 0
        first_leaf = 0
        last_leaf = 0
    elif len(leaf_nodes) == 1:
        tree_depth = 1
        root_node = 1
        first_leaf = 1
        last_leaf = 1
    else:
        tree_depth = 2
        root_node = 1
        first_leaf = 2
        last_leaf = 1 + len(leaf_nodes)

    def write_node(node_idx: int, f_link: int, b_link: int, kind: int, height: int, node_recs: list[bytes]):
        node = bytearray(node_size)
        struct.pack_into(">IIbBH", node, 0, f_link, b_link, kind, height, len(node_recs))
        offset = 14
        offsets = [offset]
        for r in node_recs:
            node[offset : offset + len(r)] = r
            offset += len(r)
            offsets.append(offset)
        # Write offsets in reverse order at the end of the node
        for i, off in enumerate(offsets):
            struct.pack_into(">H", node, node_size - 2 * (i + 1), off)
        buf[node_idx * node_size : (node_idx + 1) * node_size] = node

    # Build Header Node (node 0)
    # Record 0: BTHeaderRec (106 bytes)
    # treeDepth(H), rootNode(I), leafRecords(I), firstLeafNode(I), lastLeafNode(I),
    # nodeSize(H), maxKeyLength(H), totalNodes(I), freeNodes(I), reserved1(H),
    # clumpSize(I), btreeType(B), keyCompareType(B), attributes(I), reserved3(16*I)
    free_nodes = total_nodes - used_nodes
    attributes = 0x00000002 | 0x00000004  # kBTBigKeysMask | kBTVariableIndexKeysMask
    bt_header = struct.pack(
        ">HIIIIHHIIHIBBI",
        tree_depth,
        root_node,
        len(records),
        first_leaf,
        last_leaf,
        node_size,
        max_key_len,
        total_nodes,
        free_nodes,
        0,
        total_nodes * node_size,
        0,
        key_compare_type,
        attributes,
    ) + (b"\x00" * 64)

    # Record 1: User Data Record (128 bytes)
    user_data = b"\x00" * 128

    # Record 2: Map Record (node_size - 256 bytes)
    map_size = node_size - 256
    bitmap = bytearray(map_size)
    for n in range(used_nodes):
        bitmap[n // 8] |= 1 << (7 - (n % 8))

    write_node(0, 0, 0, 1, 0, [bt_header, user_data, bytes(bitmap)])

    # Write Leaf Nodes (and Index Node if needed)
    if len(leaf_nodes) == 1:
        write_node(1, 0, 0, -1, 1, leaf_nodes[0])
    elif len(leaf_nodes) > 1:
        index_recs = []
        for idx, l_recs in enumerate(leaf_nodes):
            node_num = first_leaf + idx
            f_link = node_num + 1 if idx + 1 < len(leaf_nodes) else 0
            b_link = node_num - 1 if idx > 0 else 0
            write_node(node_num, f_link, b_link, -1, 1, l_recs)
            # Extract key from first record in leaf
            first_rec = l_recs[0]
            key_len = struct.unpack_from(">H", first_rec, 0)[0]
            key_bytes = first_rec[: 2 + key_len]
            if len(key_bytes) % 2:
                key_bytes += b"\x00"
            index_recs.append(key_bytes + struct.pack(">I", node_num))
        write_node(1, 0, 0, 0, 2, index_recs)

    return bytes(buf)


def build_hfs_volume(staging_dir: Path, vol_name: str) -> bytes:
    now_hfs = to_hfs_time(time.time())

    # Collect filesystem entries recursively
    next_cnid = 16
    entries = []  # (parent_cnid, name, cnid, path, st, is_dir, is_symlink)

    def walk_dir(dir_path: Path, parent_cnid: int):
        nonlocal next_cnid
        for child_name in sorted(os.listdir(dir_path)):
            child_path = dir_path / child_name
            st = os.lstat(child_path)
            cnid = next_cnid
            next_cnid += 1
            is_symlink = stat.S_ISLNK(st.st_mode)
            is_dir = stat.S_ISDIR(st.st_mode) and not is_symlink
            entries.append((parent_cnid, child_name, cnid, child_path, st, is_dir, is_symlink))
            if is_dir:
                walk_dir(child_path, cnid)

    walk_dir(staging_dir, 2)

    # Compute child counts (valence) per folder CNID
    valence_map = {2: 0}
    for parent_cnid, name, cnid, path, st, is_dir, is_symlink in entries:
        valence_map[parent_cnid] = valence_map.get(parent_cnid, 0) + 1
        if is_dir:
            valence_map[cnid] = 0

    # Prepare file payloads (data fork for files and symlinks)
    # Reserve blocks:
    # Block 0: Volume Header (at byte 1024..1536)
    # Block 1: Allocation Bitmap
    # Block 2..5: Extents Overflow B-Tree (16 KB)
    # Block 6..21: Catalog B-Tree (64 KB)
    next_alloc_block = 22
    file_allocations = {}  # cnid -> (logical_size, start_block, block_count, payload_bytes)
    file_count = 0
    folder_count = 0

    for parent_cnid, name, cnid, path, st, is_dir, is_symlink in entries:
        if is_dir:
            folder_count += 1
            continue
        file_count += 1
        if is_symlink:
            target = os.readlink(path).encode("utf-8")
            data = target
        else:
            data = path.read_bytes()
        logical_size = len(data)
        if logical_size == 0:
            file_allocations[cnid] = (0, 0, 0, b"")
        else:
            blocks = math.ceil(logical_size / BLOCK_SIZE)
            file_allocations[cnid] = (logical_size, next_alloc_block, blocks, data)
            next_alloc_block += blocks

    # Build Catalog Records (key_sort_tuple, record_bytes)
    cat_items = []

    def add_folder_record(parent_id: int, folder_name: str, folder_id: int, mode: int):
        key = make_catalog_key(parent_id, folder_name)
        if len(key) % 2:
            key += b"\x00"
        bsd_info = struct.pack(">IIBBH", 0, 0, 0, 0, mode) + struct.pack(">I", 0)
        user_info = b"\x00" * 16
        finder_info = b"\x00" * 16
        val = valence_map.get(folder_id, 0)
        rec_data = (
            struct.pack(
                ">hHIIIIIII",
                1,  # kHFSPlusFolderRecord
                0,
                val,
                folder_id,
                now_hfs,
                now_hfs,
                now_hfs,
                0,
                0,
            )
            + bsd_info
            + user_info
            + finder_info
            + struct.pack(">II", 0, 0)
        )
        cat_items.append(((parent_id, hfs_fast_unicode_compare_key(folder_name)), key + rec_data))

        # Thread record for folder
        t_key = make_catalog_key(folder_id, "")
        if len(t_key) % 2:
            t_key += b"\x00"
        t_data = struct.pack(">hHI", 3, 0, parent_id) + encode_hfs_uni_str(folder_name)
        cat_items.append(((folder_id, ()), t_key + t_data))

    # Root folder (CNID 2, parent CNID 1)
    add_folder_record(1, vol_name, 2, stat.S_IFDIR | 0o755)

    for parent_cnid, name, cnid, path, st, is_dir, is_symlink in entries:
        if is_dir:
            mode = stat.S_IFDIR | (st.st_mode & 0o777)
            add_folder_record(parent_cnid, name, cnid, mode)
        else:
            key = make_catalog_key(parent_cnid, name)
            if len(key) % 2:
                key += b"\x00"
            if is_symlink:
                mode = stat.S_IFLNK | 0o755
                user_info = b"slnk" + b"rhap" + b"\x00" * 8
            else:
                mode = stat.S_IFREG | (st.st_mode & 0o777)
                user_info = b"\x00" * 16
            bsd_info = struct.pack(">IIBBH", 0, 0, 0, 0, mode) + struct.pack(">I", 1)
            finder_info = b"\x00" * 16
            logical_size, start_block, block_count, _ = file_allocations[cnid]
            data_fork = pack_fork_data(logical_size, start_block, block_count)
            rsrc_fork = pack_fork_data(0, 0, 0)
            rec_data = (
                struct.pack(
                    ">hHIIIIIII",
                    2,  # kHFSPlusFileRecord
                    2,  # kHFSThreadExistsMask
                    0,
                    cnid,
                    now_hfs,
                    now_hfs,
                    now_hfs,
                    0,
                    0,
                )
                + bsd_info
                + user_info
                + finder_info
                + struct.pack(">II", 0, 0)
                + data_fork
                + rsrc_fork
            )
            cat_items.append(((parent_cnid, hfs_fast_unicode_compare_key(name)), key + rec_data))

            # File thread record (kHFSPlusFileThreadRecord = 4)
            t_key = make_catalog_key(cnid, "")
            if len(t_key) % 2:
                t_key += b"\x00"
            t_data = struct.pack(">hHI", 4, 0, parent_cnid) + encode_hfs_uni_str(name)
            cat_items.append(((cnid, ()), t_key + t_data))

    cat_items.sort(key=lambda x: x[0])
    cat_records = [item[1] for item in cat_items]

    extents_btree = build_btree(
        node_size=4096,
        total_nodes=4,
        max_key_len=10,
        key_compare_type=0,
        records=[],
    )
    catalog_btree = build_btree(
        node_size=4096,
        total_nodes=16,
        max_key_len=516,
        key_compare_type=0xCF,  # kHFSCaseFold
        records=cat_records,
    )

    # Total blocks (+1 for trailing alternate volume header block)
    total_blocks = max(next_alloc_block + 16, 512)
    vol_bytes = bytearray(total_blocks * BLOCK_SIZE)

    # Write Allocation Bitmap (Block 1)
    alloc_bitmap = bytearray(BLOCK_SIZE)
    for b in range(next_alloc_block):
        alloc_bitmap[b // 8] |= 1 << (7 - (b % 8))
    # Also mark last block (alternate volume header) as allocated
    last_b = total_blocks - 1
    alloc_bitmap[last_b // 8] |= 1 << (7 - (last_b % 8))
    vol_bytes[1 * BLOCK_SIZE : 2 * BLOCK_SIZE] = alloc_bitmap

    # Write Extents B-Tree (Blocks 2..5)
    vol_bytes[2 * BLOCK_SIZE : 6 * BLOCK_SIZE] = extents_btree

    # Write Catalog B-Tree (Blocks 6..21)
    vol_bytes[6 * BLOCK_SIZE : 6 * BLOCK_SIZE + len(catalog_btree)] = catalog_btree

    # Write File Data Forks
    for cnid, (logical_size, start_block, block_count, payload) in file_allocations.items():
        if block_count > 0:
            off = start_block * BLOCK_SIZE
            vol_bytes[off : off + len(payload)] = payload

    # Build HFSPlusVolumeHeader (512 bytes)
    free_blocks = total_blocks - next_alloc_block - 1
    vh = bytearray(512)
    struct.pack_into(
        ">HHIIIIIIIIIIIIIIIIII",
        vh,
        0,
        0x482B,  # 'H+' signature
        4,  # version
        0x00000100,  # kHFSVolumeUnmountedMask
        0x4846534A,  # lastMountedVersion ('HFSJ')
        0,
        now_hfs,
        now_hfs,
        0,
        now_hfs,
        file_count,
        folder_count,
        BLOCK_SIZE,
        total_blocks,
        free_blocks,
        next_alloc_block,
        65536,  # rsrcClumpSize
        65536,  # dataClumpSize
        next_cnid,  # nextCatalogID
        1,  # writeCount
        1,  # encodingsBitmap
    )
    # allocationFile forkData at offset 112 (80 bytes)
    vh[112:192] = pack_fork_data(BLOCK_SIZE, 1, 1)
    # extentsFile forkData at offset 192 (80 bytes)
    vh[192:272] = pack_fork_data(len(extents_btree), 2, len(extents_btree) // BLOCK_SIZE)
    # catalogFile forkData at offset 272 (80 bytes)
    vh[272:352] = pack_fork_data(len(catalog_btree), 6, len(catalog_btree) // BLOCK_SIZE)
    # attributesFile (352..432) and startupFile (432..512) remain zeros

    # Write primary Volume Header at offset 1024 and alternate at total_size - 1024
    vol_bytes[1024:1536] = vh
    vol_bytes[len(vol_bytes) - 1024 : len(vol_bytes) - 512] = vh

    return bytes(vol_bytes)


def wrap_udzo_dmg(hfs_volume: bytes, vol_name: str, output_path: Path) -> None:
    total_sectors = len(hfs_volume) // SECTOR_SIZE
    sectors_per_chunk = 2048  # 1 MB per chunk

    data_fork = bytearray()
    runs = []

    sector_idx = 0
    while sector_idx < total_sectors:
        count = min(sectors_per_chunk, total_sectors - sector_idx)
        chunk = hfs_volume[sector_idx * SECTOR_SIZE : (sector_idx + count) * SECTOR_SIZE]
        if all(b == 0 for b in chunk):
            # Zero-fill run (type 0x00000002)
            runs.append((0x00000002, sector_idx, count, len(data_fork), 0))
        else:
            comp = zlib.compress(chunk, 9)
            runs.append((0x80000005, sector_idx, count, len(data_fork), len(comp)))
            data_fork.extend(comp)
        sector_idx += count

    # Terminator run (0xFFFFFFFF)
    runs.append((0xFFFFFFFF, total_sectors, 0, len(data_fork), 0))

    # Build mish block (204-byte header + 40 bytes per run)
    vol_crc = zlib.crc32(hfs_volume) & 0xFFFFFFFF
    mish = bytearray(204 + 40 * len(runs))
    struct.pack_into(
        ">4sIIQQQIIIIII",
        mish,
        0,
        b"mish",
        1,  # version
        0,  # first sector
        total_sectors,  # sector count
        0,  # data offset
        sectors_per_chunk,  # buffers needed
        0,  # block descriptor
        0,
        0,
        0,
        0,
        0,
    )
    # Checksum struct at offset 60: type=2 (CRC32), bit_size=32, data[0]=vol_crc
    struct.pack_into(">III", mish, 60, 2, 32, vol_crc)
    # Number of block chunks at offset 200
    struct.pack_into(">I", mish, 200, len(runs))

    for i, (rtype, s_num, s_cnt, c_off, c_len) in enumerate(runs):
        struct.pack_into(">IIQQQQ", mish, 204 + i * 40, rtype, 0, s_num, s_cnt, c_off, c_len)

    plist_dict = {
        "resource-fork": {
            "blkx": [
                {
                    "Attributes": "0x0050",
                    "CFName": f"{vol_name} (Apple_HFS : 0)",
                    "Data": bytes(mish),
                    "ID": "-1",
                    "Name": f"{vol_name} (Apple_HFS : 0)",
                }
            ],
            "plst": [
                {
                    "Attributes": "0x0050",
                    "Data": b"\x00" * 1032,
                    "ID": "0",
                    "Name": "",
                }
            ],
        }
    }
    xml_bytes = plistlib.dumps(plist_dict, fmt=plistlib.FMT_XML)

    # Build 512-byte koly trailer
    mish_crc = zlib.crc32(mish) & 0xFFFFFFFF
    koly = bytearray(512)
    struct.pack_into(
        ">4sIIIIIQQQQQQI",
        koly,
        0,
        b"koly",
        4,  # version
        512,  # header size
        1,  # flags
        0,  # running data fork offset
        0,  # data fork offset (uint64 at 24..32)
        len(data_fork),  # data fork length
        0,  # rsrc fork offset
        0,  # rsrc fork length
        1,  # segment number
        1,  # segment count
        0,  # segment ID part 1
        0,
    )
    # Exact koly offsets per Apple UDIF spec:
    koly = bytearray(512)
    struct.pack_into(">4sIII", koly, 0, b"koly", 4, 512, 1)
    struct.pack_into(">QQQQQQ", koly, 16, 0, 0, len(data_fork), 0, 0, 0)
    struct.pack_into(">II", koly, 64, 1, 1)
    # Data fork checksum at offset 88 (type=2, size=32, crc)
    struct.pack_into(">III", koly, 88, 2, 32, vol_crc)
    # XML offset and length at offset 216
    struct.pack_into(">QQ", koly, 216, len(data_fork), len(xml_bytes))
    # Master checksum at offset 352
    struct.pack_into(">III", koly, 352, 2, 32, mish_crc)
    # Image variant (1 = UDIF device image) and Sector count at offset 488..500
    struct.pack_into(">IQ", koly, 488, 1, total_sectors)

    output_path.write_bytes(bytes(data_fork) + xml_bytes + bytes(koly))


def main() -> int:
    if len(sys.argv) < 4:
        print("Usage: make_dmg.py <staging_dir> <vol_name> <output.dmg>", file=sys.stderr)
        return 1
    staging_dir = Path(sys.argv[1])
    vol_name = sys.argv[2]
    output_path = Path(sys.argv[3])
    hfs_vol = build_hfs_volume(staging_dir, vol_name)
    wrap_udzo_dmg(hfs_vol, vol_name, output_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
