#![allow(non_snake_case)]

use std::io::prelude::*;
use std::fs::File;
use std::path::Path;
use std::error::Error;

use crate::emulator::{mbc, PPU, APU, MODE, PPU_MODE};

const TIMA_SPEED: [u16; 4] = [512, 8, 32, 128];

pub struct Cartridge {
    rom: Box<dyn mbc::MemoryBankController>,
    pub bootrom: Vec<u8>,
    pub bootrom_enable: bool,
    pub title: String,
    pub gb_cart_type: MODE
}

impl Cartridge {
    fn new() -> Cartridge {
        Cartridge {
            rom: mbc::dummyMBC::new(vec![]),
            bootrom: vec![],
            bootrom_enable: false,
            title: String::new(),
            gb_cart_type: MODE::DMG
        }
    }

    #[inline]
    fn read_rom(&mut self, addr: u16) -> u8 {
        self.rom.read_rom(addr)
    }

    #[inline]
    fn write_rom(&mut self, addr: u16, val: u8) {
        self.rom.write_rom(addr, val)
    }

    #[inline]
    fn read_ram(&mut self, addr: u16) -> u8 {
        self.rom.read_ram(addr)
    }

    #[inline]
    fn write_ram(&mut self, addr: u16, val: u8) {
        self.rom.write_ram(addr, val)
    }

    pub fn load_bootrom(&mut self, p: &Path) -> Result<MODE, Box<dyn Error>> {
        let mut file = File::open(p)?;
        let mut data: Vec<u8> = vec![];
        file.read_to_end(&mut data)?;
        
        if data.len() != 0x100 && data.len() != 0x900 {
            panic!("Invalid bootrom");
        }
        self.bootrom = data;
        self.bootrom_enable = true;

        if self.bootrom.len() == 0x100 {
            Ok(MODE::DMG)
        } else {
            Ok(MODE::CGB)
        }
    }

    pub fn load_from_vec(&mut self, v: Vec<u8>) {
        self.rom = mbc::dummyMBC::new(v)
    }

    pub fn load_from_file(&mut self, p: &Path) -> Result<MODE, Box<dyn Error>> {
        let mut file = File::open(p)?;
        let mut data: Vec<u8> = vec![];
        file.read_to_end(&mut data)?;

        let mode = self.interprete_header(data)?;
        self.gb_cart_type = mode;

        Ok(mode)
    }

    fn interprete_header(&mut self, data: Vec<u8>) -> Result<MODE, &str> {
        if data.len() > 0x14F {
            if data[0x014D] != Cartridge::calculate_header_checksum(&data) {
                return Err(&"Invalid ROM header checksum")
            }

            let cgb_mode = data[0x143];
            self.title = Cartridge::get_title(&data);
            match data[0x147] {
                0x00 => {
                    self.rom = mbc::noMBC::new(data);
                },
                0x01 ..= 0x03 => {
                    self.rom = mbc::MBC1::new(data)?;
                },
                0x05 | 0x06 => {
                    self.rom = mbc::MBC2::new(data)?;
                },
                0x0F ..= 0x13 => {
                    self.rom = mbc::MBC3::new(data)?;
                },
                0x19 ..= 0x1E => {
                    self.rom = mbc::MBC5::new(data)?;
                }
                _ => panic!("{:x} - unsupported cartridge type", data[0x147])
            };

            if cgb_mode == 0x80 || cgb_mode == 0xC0 {
                Ok(MODE::CGB)
            } else {
                Ok(MODE::DMG)
            }
        } else {
            Err(&"ROM too small")
        }
    }

    fn get_title(data: &Vec<u8>) -> String {
        let mut t = String::new();
        for i in 0x134 ..= 0x13E {
            if data[i] == 0 { break; }
            t.push(data[i] as char);
        }
        t
    }

    fn calculate_header_checksum(data: &Vec<u8>) -> u8 {
        let mut sum: u8 = 0;

        for i in 0x134 ..= 0x014C {
            sum = sum.wrapping_sub(data[i as usize]).wrapping_sub(1);
        }

        sum
    }
}

pub struct Memory {
    pub cart: Cartridge,  // ROM -> 0x0000-0x7FFF 32kB, RAM -> 0xA000-0xBFFF 8kB
    pub ppu: PPU,
    apu: APU,
    pub mode: MODE,

    vram: [u8; 16*1024],  // 0x8000 - 0x9FFF 16kB (2 banks in cgb)
    ram: [u8; 32*1024], // 0xC000 - 0xDFFF 32kB (8 banks in cgb) + echo at 0xE000 - 0xFDFF
    OAM: [u8; 160],  // 0xFE00 - 0xFE9F sprite attribute memory
    hram: [u8; 127],  // 0xFF80 - 0xFFFE high ram
    pub IF: u8,  // interrupt flag 0xFF0F
    pub IER: u8,  // interrupt enable register 0xFFFF
    vram_bank: u8,
    ram_bank: u8,

    vdma_src: u16,
    vdma_dst: u16,
    hdma5: u8,
    hdma_active: bool,
    hdma_length: u8,

    // timer registers
    DIV: u16,  // FF04
    TIMA: u8, // FF05
    TMA: u8,  // FF06
    TAC: u8,  // FF07
    tima_schedule: i8,
    last_div: u16,

    serial_control: u8,
    serial_transfer: u8,
    serial_count_interrupt: u8,

    input_select: u8,
}

impl Memory {
    pub fn new() -> Memory {
        let ppu = PPU::new();
        let apu = APU::new(&ppu.d.thread);

        Memory {
            cart: Cartridge::new(),
            ppu: ppu,
            apu: apu,
            mode: MODE::DMG,

            vram: [0; 16*1024],
            ram: [0; 32*1024],
            OAM: [0; 160],
            hram: [0; 127],
            IF: 0b11100000,
            IER: 0b11100000,
            ram_bank: 1,
            vram_bank: 0,

            vdma_src: 0,
            vdma_dst: 0,
            hdma5: 0,
            hdma_active: false,
            hdma_length: 0,

            DIV: 0,
            TIMA: 0,
            TMA: 0,
            TAC: 0b11111000,
            tima_schedule: -1,
            last_div: 0,

            serial_control: 0b01111110,
            serial_transfer: 0xFF,
            serial_count_interrupt: 0,

            input_select: 0,
        }
    }

    pub fn load_bootrom(&mut self, p: &Path) -> Result<(), Box<dyn Error>> {
        self.mode = self.cart.load_bootrom(p)?;
        self.ppu.gb_mode = self.mode;
        Ok(())
    }

    pub fn load_rom(&mut self, p: &Path) -> Result<(), Box<dyn Error>> {
        self.mode = self.cart.load_from_file(p)?;
        self.ppu.gb_mode = self.mode;
        Ok(())
    }

    #[inline]
    pub fn read(&mut self, addr: u16) -> u8 {
        if self.cart.bootrom_enable {
            match addr {
                0x0000 ..= 0x00FF => {
                    return self.cart.bootrom[addr as usize]
                }
                0x0201 ..= 0x08FF if self.mode == MODE::CGB => {
                    return self.cart.bootrom[addr as usize]
                }
                _ => ()
            }
        }

        match addr {
            0x0000 ..= 0x7FFF => self.cart.read_rom(addr),
            0x8000 ..= 0x9FFF => self.vram[(addr as usize&0x1FFF) + self.vram_bank as usize*0x2000],
            0xA000 ..= 0xBFFF => self.cart.read_ram(addr-0xa000),
            0xC000 ..= 0xCFFF => self.ram[(addr&0xFFF) as usize],
            0xD000 ..= 0xDFFF => self.ram[(addr as usize&0xFFF) + self.ram_bank as usize*0x1000],
            0xE000 ..= 0xFDFF => self.ram[(addr-0xe000) as usize],
            0xFE00 ..= 0xFE9F => self.OAM[(addr-0xfe00) as usize],

            // Memory mapped io
            0xFF00 => {
                match self.input_select&0x30 {
                    0x00 => 0xF,
                    0x10 => self.ppu.in_button | self.input_select,
                    0x20 => self.ppu.in_direction | self.input_select,
                    0x30 => 0xFF,
                    _ => panic!()
                }
            },
            0xFF01 => self.serial_transfer,
            0xFF02 => self.serial_control,
            0xFF04 => (self.DIV >> 8) as u8,
            0xFF05 => self.TIMA,
            0xFF06 => self.TMA,
            0xFF07 => self.TAC,
            0xFF0F => self.IF,
            0xFF10 ..= 0xFF3F => self.apu.read(addr),
            0xFF40 ..= 0xFF4B => self.ppu.read(addr),
            0xFF4F => self.vram_bank | 0xFE,
            0xFF51 => (self.vdma_src >> 8) as u8,
            0xFF52 => self.vdma_src as u8,
            0xFF53 => (self.vdma_dst >> 8) as u8,
            0xFF54 => self.vdma_dst as u8,
            0xFF55 => self.hdma5,
            0xFF68 ..= 0xFF6C if self.mode == MODE::CGB => self.ppu.read(addr),
            0xFF70 => self.ram_bank | 0xF8, // only 3 LSb used
            0xFF80 ..= 0xFFFE => self.hram[(addr-0xff80) as usize],
            0xFFFF => self.IER,
            _ => 0xFF
        }
    }

    #[inline]
    pub fn write(&mut self, addr: u16, mut val: u8) {
        match addr {
            0x0000 ..= 0x7FFF => self.cart.write_rom(addr, val),
            0x8000 ..= 0x9FFF => self.vram[(addr as usize&0x1FFF) + self.vram_bank as usize * 0x2000] = val,
            0xA000 ..= 0xBFFF => self.cart.write_ram(addr-0xA000, val),
            0xC000 ..= 0xCFFF => self.ram[(addr&0xFFF) as usize] = val,
            0xD000 ..= 0xDFFF => self.ram[(addr as usize&0xFFF) + self.ram_bank as usize*0x1000] = val,
            0xE000 ..= 0xFDFF => self.ram[(addr-0xe000) as usize] = val,
            0xFE00 ..= 0xFE9F => self.OAM[(addr-0xfe00) as usize] = val,

            // Memory mapped io
            0xFF00 => {
                self.input_select = val&0x30
            },
            0xFF01 => {
                self.serial_transfer = val;
            },
            0xFF02 => { // intercept serial 
                let v = self.read(0xFF01);
                if v != 0 {
                    print!("{}", v as char); 
                }
                self.serial_control = 0b01111110 | val;
                if val&0x80 != 0 {
                    self.serial_count_interrupt = 8;
                }
            },
            0xFF04 => {
                self.DIV = 0;
                self.TIMA = self.TMA;
                self.tima_schedule = -1;
            },
            0xFF05 => {
                if self.tima_schedule != 1 {
                    self.tima_schedule = -1;
                    self.TIMA = val
                }
            },
            0xFF06 => self.TMA = val,
            0xFF07 => self.TAC = 0b11111000 | val,
            0xFF0F => self.IF = 0b11100000 | val,
            0xFF10 ..= 0xFF3F => {
                self.apu.write(addr, val)
            }
            0xFF46 => {  // TODO: real timings, not instant
                let mut pos = (val as u16) << 8;
                loop {
                    self.OAM[pos as usize&0xFF] = self.read(pos);
                    if pos&0xFF == 0x9F { break }
                    pos += 1;
                }
            }
            0xFF40 ..= 0xFF4B => {
                self.ppu.write(addr, val)
            },
            0xFF4F if self.mode == MODE::CGB => {
                self.vram_bank = val&0x1;
            },
            0xFF50 => {
                self.cart.bootrom = vec![];
                self.cart.bootrom_enable = false;
                self.mode = self.cart.gb_cart_type;
                self.ppu.gb_mode = self.cart.gb_cart_type;
            },
            0xFF51 => {
                self.vdma_src = (self.vdma_src&0xFF) | ((val as u16) << 8);
            },
            0xFF52 => {
                self.vdma_src = (self.vdma_src&0xFF00) | (val as u16&0xF0);
            },
            0xFF53 => {
                self.vdma_dst = (self.vdma_dst&0xF0) | ((val as u16&0x1F) << 8);
            },
            0xFF54 => {
                self.vdma_dst = (self.vdma_dst&0x1F00) | (val as u16&0xF0);
            },
            0xFF55 if self.mode == MODE::CGB => {  // TODO: "real" HDMA timings
                let length = ((val as u16&0x7F)+1) * 0x10;

                if val&0x80 != 0 { // hdma
                    self.hdma_active = true;
                    self.hdma_length = val&0x7F;
                    self.hdma5 = self.hdma_length;
                } else { // gdma
                    if self.hdma_active {
                        self.hdma_active = false;
                        self.hdma5 |= 0x80;
                    } else {
                        for i in 0 .. length {
                            let v = self.read(self.vdma_src + i);
                            self.write((0x8000 | self.vdma_dst) + i, v);
                        }
                        self.hdma5 = 0xFF;
                    }
                }
            },
            0xFF68 ..= 0xFF6C if self.mode == MODE::CGB => {
                self.ppu.write(addr, val)
            },
            0xFF70 if self.mode == MODE::CGB => {
                val = val&0x07;
                if val == 0 { val = 1; }
                self.ram_bank = val;
            },
            0xFF80..=0xFFFE => {
                self.hram[(addr-0xff80) as usize] = val
            },
            0xFFFF => self.IER = 0b11100000 | val,
            _ => ()
        }
    }

    pub fn tick(&mut self) {
        let ppu_mode = self.ppu.mode;
        self.ppu.tick(&mut self.vram, &mut self.OAM, &mut self.IF, &self.input_select);
        self.apu.tick();

        if self.hdma_active {
            if ppu_mode != self.ppu.mode && self.ppu.mode == PPU_MODE::HBLANK {
                let mut offset = self.hdma_length as u16 - (self.hdma5 as u16&0x7F);
                offset = offset * 0x10;

                for i in 0 .. 0x10 {
                    let v = self.read(self.vdma_src + i + offset);
                    self.write((0x8000 | self.vdma_dst) + i + offset, v);
                    self.tick();
                    self.tick();
                }

                if self.hdma5&0x7F == 0 {
                    self.hdma_active = false;
                    self.hdma5 = 0xFF;
                } else {
                    self.hdma5 -= 1;
                }
            }
        }

        self.serial_transfer = (self.serial_transfer >> 1) | 0x80;
        if self.serial_count_interrupt > 0 {
            self.serial_count_interrupt -= 1;
            if self.serial_count_interrupt == 0 {
                self.IF |= 0x8;
            }
        }

        self.DIV = self.DIV.wrapping_add(1);

        if self.tima_schedule >= 0 {
            if self.tima_schedule <= 2 {
                self.TIMA = self.TMA;
                self.IF |= 0b00000100;
                self.last_div = self.DIV&TIMA_SPEED[self.TAC as usize&0x03];
            }
            self.tima_schedule -= 1;
        }

        let c = if self.TAC&0x4 != 0 { 0xFFFF } else { 0 };
        let b = (self.DIV&TIMA_SPEED[self.TAC as usize&0x03])&c;
        if !b & self.last_div != 0 {
            let (tmp, carry) = self.TIMA.overflowing_add(1);
            self.TIMA = tmp;
            if carry {
                self.tima_schedule = 5;
            }
        }
        self.last_div = self.DIV&TIMA_SPEED[self.TAC as usize&0x03];
    }
}
