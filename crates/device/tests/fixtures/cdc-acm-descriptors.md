# CDC-ACM descriptor expectations

Expected descriptor fields, as the host tests read them. Values follow the USB 2.0, USB Audio 2.0 (ADC-2, FMT-2) and CDC 1.2 specifications for a high-speed 2-in 2-out device with an explicit-feedback playback endpoint and a CDC-ACM console.

### AS interface 1, playback
| Field | Alt 0 | Alt 1 | Label |
|---|---|---|---|
| bLength | 9 | 9 | ADC-2 4.9.1 |
| bDescriptorType | `0x04` | `0x04` | USB2 9.6.5 |
| bInterfaceNumber | 1 | 1 | |
| bAlternateSetting | 0 | 1 | MS-UAC2 ascending order |
| bNumEndpoints | 0 | 2 | 0, 1 or 2 allowed. ADC-2 4.9.1 |
| bInterfaceClass | `0x01` | `0x01` | ADC-2 A.4 |
| bInterfaceSubClass | `0x02` AUDIOSTREAMING | `0x02` | ADC-2 A.5 |
| bInterfaceProtocol | `0x20` | `0x20` | ADC-2 A.6 |
| iInterface | index or 0 | index or 0 | ADC-2 4.9.1 |

### Class-specific AC header
| Field | Value | Label |
|---|---|---|
| bLength | 9 | ADC-2 4.7.2 Table 4-5 |
| bDescriptorType | `0x24` CS_INTERFACE | ADC-2 A.8 |
| bDescriptorSubtype | `0x01` HEADER | ADC-2 A.9 |
| bcdADC | `0x0200` | Release number in BCD. ADC-2 4.7.2. |
| bCategory | `0x08` I/O_BOX or `0x0A` PRO-AUDIO | ADC-2 A.7 for codes. |
| wTotalLength | 75 with no Feature Unit, 111 with one 2-channel Feature Unit per direction | Header plus all clock, unit and terminal descriptors. ADC-2 4.7.2. Arithmetic: 9 + 8 + 17 + 12 + 17 + 12, plus 18 per Feature Unit |
| bmControls | `0x00` | D1..0 is Latency Control. ADC-2 4.7.2 |

### Clock Source
| Field | Value | Label |
|---|---|---|
| bLength | 8 | ADC-2 4.7.2.1 Table 4-6 |
| bDescriptorType | `0x24` | ADC-2 A.8 |
| bDescriptorSubtype | `0x0A` CLOCK_SOURCE | ADC-2 A.9 |
| bClockID | 10 | |
| bmAttributes | `0x03` | Internal programmable (`11`), D2 = 0 free running, not tied to SOF. ADC-2 4.7.2.1. The asynchronous design is free running |
| bmControls | `0x07` | Frequency host programmable (`11`), Validity read-only (`01`). ADC-2 4.7.2.1. |
| bAssocTerminal | 0 | |
| iClockSource | string index or 0 | ADC-2 4.7.2.1 |

### Configuration descriptor
| Field | Value | Label |
|---|---|---|
| bLength | 9 | USB2 9.6.3 |
| bDescriptorType | `0x02` | USB2 9.6.3 |
| wTotalLength | sum of all descriptors | USB2 9.6.3. The audio part alone is 218 bytes with no Feature Unit. Arithmetic: below |
| bNumInterfaces | 3 for audio, plus CDC and MIDI | |
| bConfigurationValue | 1 | |
| bmAttributes, bMaxPower | bus or self power as the board needs | bus powered |

### Descriptor fields
| Field | Value | Rule and label |
|---|---|---|
| bLength | 7 | ADC-2 4.10.2.1 Table 4-35 |
| bDescriptorType | `0x05` ENDPOINT | ADC-2 4.10.2.1, USB2 9.6.6 |
| bEndpointAddress | `0x80` OR the OUT data endpoint number | Same number, opposite direction. USB2 9.6.6 |
| bmAttributes | `0x11` | Isochronous (`01`), No Synchronization (`00`), Feedback usage (`01`). USB2 9.6.6, ADC-2 4.10.2.1 |
| wMaxPacketSize | 4 | The 16.16 value fills four bytes. USB2 5.12.4.2. |
| bInterval | 1 to 16, period 2^(bInterval-1) microframes | USB2 9.6.6. The chosen value follows P. |

### Device descriptor
| Field | Value | Label |
|---|---|---|
| bLength | 18 | USB2 9.6.1 |
| bDescriptorType | `0x01` | USB2 9.6.1 |
| bcdUSB | `0x0200` | High-speed capable devices use 0200H. USB2 9.2.6.6. 0201H or more announces BOS. LPM-ECN. No BOS is needed here. |
| bDeviceClass, SubClass, Protocol | `0xEF`, `0x02`, `0x01` | ADC-2 4.2, MS-IAD |
| bMaxPacketSize0 | 64 | High-speed control endpoints use 64 bytes. USB2 5.5.3 |
| idVendor, idProduct | the open pid.codes pair 0x1209:0x0001 | |
| bNumConfigurations | 1 | Windows reports `USB\COMPOSITE` only for a single-configuration device. MS-COMP |

### Device qualifier
| Field | Value | Label |
|---|---|---|
| bLength | 10 | USB2 9.6.2 Table 9-9 |
| bDescriptorType | `0x06` | USB2 9.6.2 |
| bcdUSB | `0x0200` | Must be at least 0200H. USB2 9.6.2 |
| bDeviceClass, SubClass, Protocol | `0xEF`, `0x02`, `0x01` | ADC-2 4.3 |
| bMaxPacketSize0 | 64 | Full-speed control endpoints may use 8, 16, 32 or 64. USB2 5.5.3 |
| bNumConfigurations | 1 | Number of other-speed configurations. USB2 9.6.2 |
| bReserved | 0 | USB2 9.6.2 |

### Input Terminal 1, USB streaming
| Field | Value | Label |
|---|---|---|
| bLength | 17 | ADC-2 4.7.2.4 Table 4-9 |
| bDescriptorType | `0x24` | ADC-2 A.8 |
| bDescriptorSubtype | `0x02` INPUT_TERMINAL | ADC-2 A.9 |
| bTerminalID | 1 | |
| wTerminalType | `0x0101` USB streaming | TERMT-2 2.1 |
| bAssocTerminal | 0 | Zero when no association. ADC-2 4.7.2.4 |
| bCSourceID | 10 | ADC-2 4.7.2.4 |
| bNrChannels | 2 | ADC-2 4.1 |
| bmChannelConfig | `0x00000003` FL and FR | ADC-2 4.1 |
| iChannelNames | 0 | ADC-2 4.1 |
| bmControls | `0x0000` | |
| iTerminal | string index or 0 | ADC-2 4.7.2.4 |

### Interface Association Descriptor
| Field | Value | Label |
|---|---|---|
| bLength | 8 | ADC-2 4.6 Table 4-3 |
| bDescriptorType | `0x0B` INTERFACE_ASSOCIATION | IAD-ECN |
| bFirstInterface | 0 | The AC interface comes first. ADC-2 4.6 |
| bInterfaceCount | 3 | AC plus two AS. |
| bFunctionClass | `0x01` AUDIO_FUNCTION | ADC-2 A.1 gives the value as "AUDIO", the interface class code `0x01`. ADC-2 A.1, A.4 |
| bFunctionSubClass | `0x00` FUNCTION_SUBCLASS_UNDEFINED | ADC-2 4.6, A.2 |
| bFunctionProtocol | `0x20` AF_VERSION_02_00 | ADC-2 4.6, A.3, A.6 |
| iFunction | string index or 0 | ADC-2 4.6 |

### Other-speed configuration descriptor
| Field | Value | Label |
|---|---|---|
| bLength | 9 | USB2 9.6.4 Table 9-11 |
| bDescriptorType | `0x07` | OTHER_SPEED_CONFIGURATION. USB2 Table 9-5, IAD-ECN |
| bConfigurationValue | 1 | the one configuration |
| bmAttributes | `0x80` | "Same as Configuration descriptor". USB2 Table 9-11. Bus powered. |
| bMaxPower | 250 | Same as the configuration, 500 mA in 2 mA units. USB2 Table 9-11, 9.6.3 |

### Output Terminal 3, line out
| Field | Value | Label |
|---|---|---|
| bLength | 12 | ADC-2 4.7.2.5 Table 4-10 |
| bDescriptorSubtype | `0x03` OUTPUT_TERMINAL | ADC-2 A.9 |
| bTerminalID | 3 | |
| wTerminalType | `0x0603` Line connector, or `0x0301` Speaker | TERMT-2 2.3, 2.6 for codes. |
| bAssocTerminal | 0 | ADC-2 4.7.2.5 |
| bSourceID | 1, or the Feature Unit ID | ADC-2 4.7.2.5 |
| bCSourceID | 10 | ADC-2 4.7.2.5 |
| bmControls | `0x0000` | |
| iTerminal | string index or 0 | ADC-2 4.7.2.5 |

### Standard AC interface
| Field | Value | Label |
|---|---|---|
| bLength | 9 | ADC-2 4.7.1 Table 4-4 |
| bDescriptorType | `0x04` | USB2 9.6.5 |
| bInterfaceNumber | 0 | |
| bAlternateSetting | 0 | Must be 0. ADC-2 4.7.1, 3.16.1 |
| bNumEndpoints | 0, or 1 with the interrupt endpoint | ADC-2 4.7.1 |
| bInterfaceClass | `0x01` | ADC-2 A.4 |
| bInterfaceSubClass | `0x01` AUDIOCONTROL | ADC-2 A.5 |
| bInterfaceProtocol | `0x20` | ADC-2 A.6 |
| iInterface | string index or 0 | ADC-2 4.7.1 |

Class-specific AS general:
| Field | Value | Label |
|---|---|---|
| bLength | 16 | ADC-2 4.9.2 Table 4-27 |
| bDescriptorType | `0x24` | ADC-2 A.8 |
| bDescriptorSubtype | `0x01` AS_GENERAL | ADC-2 A.10 |
| bTerminalLink | 1 | The USB streaming Input Terminal. Same in every alt. ADC-2 4.9.2, MS-UAC2 |
| bmControls | `0x00` | |
| bFormatType | `0x01` FORMAT_TYPE_I | FMT-2 A.1. Must equal the Format Type descriptor. MS-UAC2 |
| bmFormats | `0x00000001` PCM | Exactly one bit. FMT-2 A.2.1, MS-UAC2 |
| bNrChannels | 2 | ADC-2 4.9.2 |
| bmChannelConfig | `0x00000003` | ADC-2 4.1 |
| iChannelNames | 0 | ADC-2 4.9.2 |

Class-specific isochronous data endpoint:
| Field | Value | Label |
|---|---|---|
| bLength | 8 | ADC-2 4.10.1.2 Table 4-34 |
| bDescriptorType | `0x25` CS_ENDPOINT | ADC-2 A.8 |
| bDescriptorSubtype | `0x01` EP_GENERAL | ADC-2 A.13 |
| bmAttributes | `0x00` | MaxPacketsOnly applies to Type II only. ADC-2 4.10.1.2 |
| bmControls | `0x00` | . usbaudio2 ignores it. MS-UAC2 |
| bLockDelayUnits | 0 | Must be zero for asynchronous endpoints. ADC-2 4.10.1.2 |
| wLockDelay | 0 | Same rule. ADC-2 4.10.1.2 |

Standard isochronous data endpoint, OUT:
| Field | Value | Label |
|---|---|---|
| bLength | 7 | ADC-2 4.10.1.1 Table 4-33 |
| bDescriptorType | `0x05` | ADC-2 4.10.1.1 |
| bEndpointAddress | `0x01` | see section 1 |
| bmAttributes | `0x05` | Isochronous (`01`), Asynchronous (`01`), Data (`00`). ADC-2 4.10.1.1, USB2 9.6.6 |
| wMaxPacketSize | `0x00C8` (200) | Bits 12..11 are zero, one transaction per microframe. USB2 9.6.6. |
| bInterval | 1 | One packet every 125 us. USB2 9.6.6. |

Type I format type:
| Field | Value | Label |
|---|---|---|
| bLength | 6 | FMT-2 2.3.1.6 Table 2-2 |
| bDescriptorType | `0x24` | FMT-2 2.3.1.6 |
| bDescriptorSubtype | `0x02` FORMAT_TYPE | ADC-2 A.10 |
| bFormatType | `0x01` | FMT-2 A.1 |
| bSubslotSize | 4 | FMT-2 2.3.1.6. 24-bit samples in 32-bit slots |
| bBitResolution | 24 | FMT-2 2.3.1.6. 24-bit samples in 32-bit slots |

### Abstract Control Management functional descriptor
| Field | Value | Label |
|---|---|---|
| bFunctionLength | 4 | PSTN-1.2 Table 4 |
| bDescriptorType | `0x24` | CDC-1.2 Table 12 |
| bDescriptorSubtype | `0x02` | CDC-1.2 Table 13 |
| bmCapabilities | `0x02` | D1 set. The device supports Set_Line_Coding, Set_Control_Line_State, Get_Line_Coding and the Serial_State notification. D0, D2 and D3 clear. PSTN-1.2 Table 4. |

### Bulk endpoints
| Field | Value | Label |
|---|---|---|
| bLength | 7 | USB2 9.6.6 |
| bDescriptorType | `0x05` | USB2 9.6.6 |
| bmAttributes | `0x02` | Bulk. USB2 9.6.6 bits 1..0 = 10 |
| bInterval | 0 | High-speed bulk uses the field for compliance only. USB2 9.6.6. |

### Call Management functional descriptor
| Field | Value | Label |
|---|---|---|
| bFunctionLength | 5 | PSTN-1.2 Table 3 |
| bDescriptorType | `0x24` | CDC-1.2 Table 12 |
| bDescriptorSubtype | `0x01` | CDC-1.2 Table 13 |
| bmCapabilities | `0x00` | D0 clear, the device does no call management. D1 is then reset to zero. PSTN-1.2 Table 3. A console has no calls |

### Communications Class interface
| Field | Value | Label |
|---|---|---|
| bLength | 9 | USB2 9.6.5 |
| bDescriptorType | `0x04` | USB2 9.6.5 |
| bAlternateSetting | 0 | one setting |
| bNumEndpoints | 1 | The notification element. CDC-1.2 3.5.1 |
| bInterfaceClass, SubClass, Protocol | `0x02`, `0x02`, `0x00` | Communications Interface Class, Abstract Control Model, no class-specific protocol. CDC-1.2 Tables 3, 4, 5. usbser.sys needs class and subclass 2. MS-SER. Protocol is "Any". MS-COLL |
| iInterface | 0 | |

### Data Class interface
| Field | Value | Label |
|---|---|---|
| bLength | 9 | USB2 9.6.5 |
| bDescriptorType | `0x04` | USB2 9.6.5 |
| bAlternateSetting | 0 | one setting |
| bNumEndpoints | 2 | One In and one Out of one type. CDC-1.2 3.5.2 |
| bInterfaceClass, SubClass, Protocol | `0x0A`, `0x00`, `0x00` | Data Interface Class. Subclass unused. No class-specific protocol. CDC-1.2 Tables 6, 7, 10, 4.6 |
| iInterface | 0 | |

### Header functional descriptor
| Field | Value | Label |
|---|---|---|
| bFunctionLength | 5 | CDC-1.2 Table 15 |
| bDescriptorType | `0x24` | CS_INTERFACE. CDC-1.2 Table 12 |
| bDescriptorSubtype | `0x00` | Header. CDC-1.2 Table 13 |
| bcdCDC | `0x0120` | Release 1.2 in BCD. CDC-1.2 5.2.3.1. The samples show `0x0110`. PSTN-1.2 Table 9 |

### Notification endpoint
| Field | Value | Label |
|---|---|---|
| bLength | 7 | USB2 9.6.6 |
| bDescriptorType | `0x05` | USB2 9.6.6 |
| bEndpointAddress | `0x83` | the endpoint map |
| bmAttributes | `0x03` | Interrupt. USB2 9.6.6 bits 1..0 = 11 |
| wMaxPacketSize | 64 | Size used. Full speed allows up to 64, high speed up to 1024. USB2 5.7.3 |

### Serial Interface Association Descriptor
| Field | Value | Label |
|---|---|---|
| bLength | 8 | IAD-ECN Table 9-Z |
| bDescriptorType | `0x0B` | IAD-ECN |
| bInterfaceCount | 2 | Communications and Data. PSTN-1.2 3.2.2 |
| bFunctionClass, SubClass, Protocol | `0x02`, `0x02`, `0x00` | The first interface's triple is the preferred value. IAD-ECN. The class must not be 0. IAD-ECN |
| iFunction | 0 | No string. |

### Union functional descriptor
| Field | Value | Label |
|---|---|---|
| bFunctionLength | 5 | One subordinate. CDC-1.2 Table 16 |
| bDescriptorType | `0x24` | CDC-1.2 Table 12 |
| bDescriptorSubtype | `0x06` | CDC-1.2 Table 13 |
