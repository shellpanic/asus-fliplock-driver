"""Linux I2C_RDWR transport, adapted from upstream NumberPad packet format."""
import ctypes
import fcntl
import os


class Message(ctypes.Structure):
    _fields_ = [('addr', ctypes.c_uint16), ('flags', ctypes.c_uint16),
                ('length', ctypes.c_uint16), ('buffer', ctypes.POINTER(ctypes.c_uint8))]


class Transfer(ctypes.Structure):
    _fields_ = [('messages', ctypes.POINTER(Message)), ('count', ctypes.c_uint32)]


def packet(adapter, value):
    data = (ctypes.c_uint8 * 13)(5, 0, 0x3d, 3, 6, 0, 7, 0, 0x0d, 0x14, 3,
                                value, 0xad)
    message = Message(0x15, 0, len(data), data)
    transfer = Transfer(ctypes.pointer(message), 1)
    fd = os.open(adapter, os.O_RDWR | os.O_CLOEXEC)
    try:
        fcntl.ioctl(fd, 0x0707, transfer)  # I2C_RDWR; only the selected adapter
    finally:
        os.close(fd)


def illuminate(adapter, enabled):
    if enabled:
        packet(adapter, 0x60)
    packet(adapter, 0x01 if enabled else 0x00)
