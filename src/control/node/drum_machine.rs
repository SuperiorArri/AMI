use super::{Control, ControlPtr, RequestKind};
use crate::{
    control::{command::ResponseCallback, CtrSender},
    json::{deser_field, deser_field_opt, serialize, DeserializationResult, SerializationResult},
    midi,
    path::VirtualPaths,
    rhythm::Rhythm,
};
use axum::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{fs, path::Path, time::Duration};

const DEFAULT_NAME: &str = "Drum Machine";

pub struct Node {
    name: String,
    enabled: bool,
    voices: Voices,
    rhythm: Option<Rhythm>,
    sender: Option<CtrSender>,
    virtual_paths: Option<VirtualPaths>,
}

impl Node {
    fn set_name(&mut self, name: String) -> JsonUpdateKind {
        self.name = name.clone();
        update_fields_or_fail(|updates| {
            updates.push(("name".to_owned(), serialize(name)?));
            Ok(())
        })
    }

    fn set_enabled(&mut self, flag: bool) -> JsonUpdateKind {
        self.enabled = flag;
        update_fields_or_fail(|updates| {
            updates.push(("enabled".to_owned(), serialize(flag)?));
            Ok(())
        })
    }

    fn add_voice(&mut self) -> JsonUpdateKind {
        self.voices.add_voice();
        update_fields_or_fail(|updates| {
            updates.push(("voices".into(), serialize(&self.voices)?));
            Ok(())
        })
    }

    fn remove_voice(&mut self, index: usize) -> JsonUpdateKind {
        if self.voices.remove_voice(index).is_ok() {
            update_fields_or_fail(|updates| {
                updates.push(("voices".into(), serialize(&self.voices)?));
                Ok(())
            })
        } else {
            JsonUpdateKind::Failed
        }
    }

    fn clear_voices(&mut self) -> JsonUpdateKind {
        self.voices.clear();
        update_fields_or_fail(|updates| {
            updates.push(("voices".into(), serialize(&self.voices)?));
            Ok(())
        })
    }

    fn set_voice_name(&mut self, voice_index: usize, name: String) -> JsonUpdateKind {
        let res = self.voices.set_voice_name(voice_index, name).is_ok();
        if res {
            update_fields_or_fail(|updates| {
                updates.push(("voices".into(), serialize(&self.voices)?));
                Ok(())
            })
        } else {
            JsonUpdateKind::Failed
        }
    }

    fn set_voice_instrument(
        &mut self,
        voice_index: usize,
        instrument_index: Option<usize>,
    ) -> JsonUpdateKind {
        let res = self
            .voices
            .set_voice_instrument(voice_index, instrument_index)
            .is_ok();
        if res {
            update_fields_or_fail(|updates| {
                updates.push(("voices".into(), serialize(&self.voices)?));
                Ok(())
            })
        } else {
            JsonUpdateKind::Failed
        }
    }

    fn set_voice_note(&mut self, voice_index: usize, note: u8) -> JsonUpdateKind {
        if self.voices.set_voice_note(voice_index, note).is_ok() {
            update_fields_or_fail(|updates| {
                updates.push(("voices".into(), serialize(&self.voices)?));
                Ok(())
            })
        } else {
            JsonUpdateKind::Failed
        }
    }

    fn set_voice_velocity(&mut self, voice_index: usize, velocity: u8) -> JsonUpdateKind {
        if self
            .voices
            .set_voice_velocity(voice_index, velocity)
            .is_ok()
        {
            update_fields_or_fail(|updates| {
                updates.push(("voices".into(), serialize(&self.voices)?));
                Ok(())
            })
        } else {
            JsonUpdateKind::Failed
        }
    }

    fn set_slot(&mut self, voice_index: usize, slot_index: usize, enabled: bool) -> JsonUpdateKind {
        let res = self
            .voices
            .set_slot(voice_index, slot_index, enabled)
            .is_ok();
        if res {
            update_fields_or_fail(|updates| {
                updates.push(("voices".into(), serialize(&self.voices)?));
                Ok(())
            })
        } else {
            JsonUpdateKind::Failed
        }
    }

    fn set_rhythm(&mut self, rhythm: Rhythm) -> JsonUpdateKind {
        self.rhythm = rhythm;
        self.voices.set_num_slots(self.rhythm.num_slots());
        update_fields_or_fail(|updates| {
            updates.push(("rhythm".to_owned(), serialize(rhythm)?));
            updates.push(("voices".into(), serialize(&self.voices)?));
            Ok(())
        })
    }

    fn set_tempo_bpm(&mut self, tempo_bpm: f32) -> JsonUpdateKind {
        self.tempo_bpm = tempo_bpm;
        update_fields_or_fail(|updates| {
            updates.push(("tempo_bpm".to_owned(), serialize(tempo_bpm)?));
            Ok(())
        })
    }

    fn slot_index(&self, beat_num: u8, div_num: u8) -> usize {
        beat_num as usize * self.rhythm.num_divs as usize + div_num as usize
    }

    async fn beat_tick(&mut self, beat_num: u8, div_num: u8) {
        let slot_index = self.slot_index(beat_num, div_num);
        for voice in &self.voices.voices {
            if let Some(instrument_index) = &voice.instrument_index {
                let channel = voice.channel;
                if slot_index < voice.slots.len() {
                    let enabled = voice.slots[slot_index];
                    if enabled {
                        self.produce_noise(*instrument_index, channel, voice.note, voice.velocity)
                            .await;
                    }
                }
            }
        }
    }

    async fn produce_noise(&self, instrument_id: usize, channel: u8, note: u8, velocity: u8) {
        _ = self
            .sender
            .send(ControlMessage {
                instrument_id,
                channel,
                note,
                velocity,
            })
            .await;
        _ = self
            .sender
            .send(ControlMessage {
                instrument_id,
                channel,
                note,
                velocity: 0,
            })
            .await;
    }

    pub async fn tick(&mut self) {
        self.receive_requests();
        if self.enabled {
            let time = self.timestamp();
            let period = self.period();
            if time - self.last_time >= period {
                self.beat_tick(self.current_beat, self.current_div).await;
                self.advance_div();
                self.last_time += period;
            }
        }
    }

    pub fn period(&self) -> f32 {
        60.0 / (self.tempo_bpm * self.rhythm.num_divs as f32)
    }

    fn advance_div(&mut self) {
        self.current_div = (self.current_div + 1) % self.rhythm.num_divs;
        if self.current_div == 0 {
            self.advance_beat();
        }
    }

    fn advance_beat(&mut self) {
        self.current_beat = (self.current_beat + 1) % self.rhythm.num_beats;
    }

    fn timestamp(&self) -> f32 {
        self.start.elapsed().unwrap_or(Duration::ZERO).as_secs_f32()
    }

    fn receive_requests(&mut self) {
        while let Ok((kind, responder)) = self.req_rx.try_recv() {
            let update = self.process_request(kind);
            if let Err(e) = responder.send(update) {
                tracing::error!("Failed to send a response: {e:?}");
            }
        }
    }

    fn load_preset_from_file(&mut self, path: &Path) -> JsonUpdateKind {
        if let Some(path) = self.virtual_paths.translate(path) {
            if let Ok(file) = fs::read_to_string(path) {
                if let Ok(source) = serde_json::from_str(&file) {
                    if self.deserialize_preset(&source).is_ok() {
                        self.reset();
                        return update_fields_or_fail(|updates| {
                            updates.push((
                                "rhythm".to_owned(),
                                serialize(&self.rhythm.ok_or(SerializationError)?)?,
                            ));
                            updates.push(("voices".into(), serialize(&self.voices)?));
                            updates.push(("tempo_bpm".into(), serialize(self.tempo_bpm)?));
                            Ok(())
                        });
                    }
                }
            }
        }
        JsonUpdateKind::Failed
    }

    fn save_preset_to_file(&self, path: &Path) -> JsonUpdateKind {
        if let Some(path) = self.virtual_paths.translate(path) {
            if let Ok(source) = self.serialize_preset() {
                if let Ok(source) = serde_json::to_string_pretty(&source) {
                    if fs::write(path, source).is_ok() {
                        return JsonUpdateKind::Ok;
                    }
                }
            }
        }
        JsonUpdateKind::Failed
    }

    fn deserialize_preset(&mut self, source: &serde_json::Value) -> DeserializationResult {
        deser_field(source, "voices", |v| self.voices = v)?;
        deser_field(source, "rhythm", |v| self.rhythm = v)?;
        deser_field(source, "tempo_bpm", |v| self.tempo_bpm = v)?;
        Ok(())
    }

    fn serialize_preset(&self) -> SerializationResult {
        let result: serde_json::Value = json!({
            "voices": serialize(&self.voices)?,
            "rhythm": serialize(&self.rhythm.ok_or(SerializationError)?)?,
            "tempo_bpm": serialize(self.tempo_bpm)?,
        });
        Ok(result)
    }

    fn process_request(&mut self, kind: RequestKind) -> JsonUpdateKind {
        match kind {
            RequestKind::SetEnabled(flag) => self.set_enabled(flag),
            RequestKind::AddVoice => self.add_voice(),
            RequestKind::RemoveVoice(index) => self.remove_voice(index),
            RequestKind::ClearVoices => self.clear_voices(),
            RequestKind::SetVoiceName(index, name) => self.set_voice_name(index, name),
            RequestKind::SetVoiceInstrument(index, ins) => self.set_voice_instrument(index, ins),
            RequestKind::SetVoiceNote(index, note) => self.set_voice_note(index, note),
            RequestKind::SetVoiceVelocity(index, veloc) => self.set_voice_velocity(index, veloc),
            RequestKind::SetSlot(vi, si, slot) => self.set_slot(vi, si, slot),
            RequestKind::SetRhythm(rhythm) => self.set_rhythm(rhythm),
            RequestKind::SetTempoBpm(tempo_bpm) => self.set_tempo_bpm(tempo_bpm),
            RequestKind::LoadPreset(path) => self.load_preset_from_file(&path),
            RequestKind::SavePreset(path) => self.save_preset_to_file(&path),
        }
    }
}

impl Default for Node {
    fn default() -> Self {
        Self {
            name: DEFAULT_NAME.into(),
            enabled: true,
            voices: Default::default(),
            rhythm: None,
            sender: None,
            virtual_paths: None,
        }
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            enabled: self.enabled,
            voices: self.voices.clone(),
            rhythm: self.rhythm,
            sender: None,
            virtual_paths: self.virtual_paths.clone(),
        }
    }
}

#[async_trait]
impl Control for Node {
    async fn reset(&mut self) {}

    async fn beat_tick(&mut self, beat_num: u8, div_num: u8) {
        todo!()
    }

    fn set_virtual_paths(&mut self, vp: VirtualPaths) {
        self.virtual_paths = Some(vp);
    }

    fn set_rhythm(&mut self, rhythm: Rhythm) {
        self.rhythm = Some(rhythm);
        self.voices.set_num_slots(rhythm.num_slots());
    }

    fn set_tempo_bpm(&mut self, _tempo_bpm: f32) {}

    fn receive_midi_message(&mut self, _message: &midi::Message) {}

    fn set_control_sender(&mut self, sender: CtrSender) {
        self.sender = Some(sender);
    }

    fn set_json_updater(&mut self, _updater: JsonUpdater) {}

    fn process_request(&mut self, kind: RequestKind, cb: ResponseCallback) {
        match kind {
            RequestKind::SetName(_) => todo!(),
            RequestKind::SetEnabled(_) => todo!(),
            RequestKind::LoadPreset(_) => todo!(),
            RequestKind::SavePreset(_) => todo!(),
            RequestKind::SetUserPreset(_) => todo!(),
            RequestKind::SetUserPresetEnabled(_, _) => todo!(),
            RequestKind::AddVoice => todo!(),
            RequestKind::RemoveVoice(_) => todo!(),
            RequestKind::ClearVoices => todo!(),
            RequestKind::SetVoiceName(_, _) => todo!(),
            RequestKind::SetVoiceInstrument(_, _) => todo!(),
            RequestKind::SetVoiceNote(_, _) => todo!(),
            RequestKind::SetVoiceVelocity(_, _) => todo!(),
            RequestKind::SetSlot(_, _, _) => todo!(),
            RequestKind::SetRhythm(_) => todo!(),
            RequestKind::SetTempoBpm(_) => todo!(),
        }
    }

    fn serialize(&self) -> SerializationResult {
        let result: serde_json::Value = json!({
            "enabled": serialize(self.enabled)?,
            "voices": serialize(&self.voices)?,
        });
        Ok(result)
    }

    fn deserialize(&mut self, source: &serde_json::Value) -> DeserializationResult {
        deser_field_opt(source, "enabled", |v| self.enabled = v)?;
        deser_field_opt(source, "voices", |v| self.voices = v)?;
        Ok(())
    }

    fn clone_node(&self) -> ControlPtr {
        Box::new(self.clone())
    }
}

fn interpolate_slots(voice: &mut Voice, factor: usize) {
    let mut interpolated = Vec::with_capacity(voice.slots.len() * factor);
    for item in voice.slots.iter() {
        interpolated.push(*item);
        interpolated.extend(std::iter::repeat(false).take(factor - 1));
    }
    voice.slots = interpolated;
}

fn decimate_slots(voice: &mut Voice, factor: usize) {
    let mut decimated = Vec::with_capacity(voice.slots.len() / factor);
    for item in voice.slots.iter().step_by(factor) {
        decimated.push(*item);
    }
    voice.slots = decimated;
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Voices {
    num_slots: usize,
    voices: Vec<Voice>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Voice {
    pub name: String,
    pub instrument_index: Option<usize>,
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
    slots: Vec<bool>,
}

impl Voices {
    pub fn set_num_slots(&mut self, num_slots: usize) {
        let prev_num_slots = self.num_slots;
        self.num_slots = num_slots;
        self.update_slots(prev_num_slots);
    }

    pub fn add_voice(&mut self) {
        self.voices.push(Voice {
            name: String::new(),
            instrument_index: None,
            channel: 9,
            note: 0,
            velocity: 127,
            slots: vec![false; self.num_slots],
        });
    }

    pub fn remove_voice(&mut self, index: usize) -> Result<(), ()> {
        if index < self.voices.len() {
            self.voices.remove(index);
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn clear(&mut self) {
        self.voices.clear();
    }

    pub fn set_voice_name(&mut self, voice_index: usize, name: String) -> Result<(), ()> {
        if voice_index < self.voices.len() {
            self.voices[voice_index].name = name;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn set_voice_instrument(
        &mut self,
        voice_index: usize,
        instrument_index: Option<usize>,
    ) -> Result<(), ()> {
        if voice_index < self.voices.len() {
            self.voices[voice_index].instrument_index = instrument_index;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn set_voice_note(&mut self, voice_index: usize, note: u8) -> Result<(), ()> {
        if voice_index < self.voices.len() {
            self.voices[voice_index].note = note;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn set_voice_velocity(&mut self, voice_index: usize, velocity: u8) -> Result<(), ()> {
        if voice_index < self.voices.len() {
            self.voices[voice_index].velocity = velocity;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn set_slot(
        &mut self,
        voice_index: usize,
        slot_index: usize,
        enabled: bool,
    ) -> Result<(), ()> {
        if voice_index < self.voices.len() {
            let voice = &mut self.voices[voice_index];
            if slot_index < voice.slots.len() {
                voice.slots[slot_index] = enabled;
                Ok(())
            } else {
                Err(())
            }
        } else {
            Err(())
        }
    }

    pub fn set_all_to_silence(&mut self) {
        self.voices
            .iter_mut()
            .for_each(|voice| voice.instrument_index = None);
    }

    pub fn reindex_instruments(&mut self, removed_index: usize) {
        self.voices
            .iter_mut()
            .for_each(|voice| match voice.instrument_index {
                Some(instr_index) if instr_index == removed_index => voice.instrument_index = None,
                Some(instr_index) if instr_index > removed_index => {
                    voice.instrument_index = Some(instr_index - 1);
                }
                _ => {}
            });
    }

    fn update_slots(&mut self, prev_num_slots: usize) {
        let num_slots = self.num_slots;
        if prev_num_slots == 0 || num_slots == 0 {
            self.update_slots_resize(num_slots);
        } else if num_slots > prev_num_slots {
            if num_slots % prev_num_slots == 0 {
                self.update_slots_interleave(num_slots / prev_num_slots);
            } else {
                self.update_slots_append(num_slots - prev_num_slots)
            }
        } else if num_slots < prev_num_slots {
            if prev_num_slots % num_slots == 0 {
                self.update_slots_decimate(prev_num_slots / num_slots);
            } else {
                //FIXME: attempt to subtract with overflow
                self.update_slots_cut_out(prev_num_slots - num_slots)
            }
        }
    }

    fn update_slots_interleave(&mut self, factor: usize) {
        self.voices
            .iter_mut()
            .for_each(|voice| interpolate_slots(voice, factor));
    }

    fn update_slots_append(&mut self, number: usize) {
        self.voices
            .iter_mut()
            .for_each(|voice| voice.slots.resize(voice.slots.len() + number, false));
    }

    fn update_slots_decimate(&mut self, factor: usize) {
        self.voices
            .iter_mut()
            .for_each(|voice| decimate_slots(voice, factor));
    }

    fn update_slots_cut_out(&mut self, number: usize) {
        self.voices
            .iter_mut()
            .for_each(|voice| voice.slots.resize(voice.slots.len() - number, false));
    }

    fn update_slots_resize(&mut self, size: usize) {
        self.voices
            .iter_mut()
            .for_each(|voice| voice.slots.resize(size, false));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    pub fn interpolate_decimate_slots() {
        //TODO: write new test

        // let v1 = DrumMachineNoise {
        //     instrument_index: 0,
        //     note: 0,
        //     velocity: 0,
        // };

        // let v2 = DrumMachineNoise {
        //     instrument_index: 1,
        //     note: 0,
        //     velocity: 0,
        // };

        // let values = vec![Some(v1.clone()), Some(v2.clone())];
        // let interpolated_values = super::interpolate_slots(&values, 2);
        // let decimated_values = super::decimate_slots(&values, 2);

        // assert_eq!(
        //     interpolated_values,
        //     vec![Some(v1.clone()), None, Some(v2.clone()), None,]
        // );

        // assert_eq!(decimated_values, vec![Some(v1.clone())]);
    }

    #[test]
    pub fn reindex_slots() {
        //TODO: write new test

        // let v1 = DrumMachineNoise {
        //     instrument_index: 0,
        //     note: 0,
        //     velocity: 0,
        // };

        // let v2 = DrumMachineNoise {
        //     instrument_index: 1,
        //     note: 0,
        //     velocity: 0,
        // };

        // let v3 = DrumMachineNoise {
        //     instrument_index: 2,
        //     note: 0,
        //     velocity: 0,
        // };

        // let values = vec![Some(v1.clone()), Some(v2.clone()), Some(v3.clone())];
        // assert_eq!(
        //     super::reindex_slots(&values, 0),
        //     vec![None, Some(v1.clone()), Some(v2.clone())]
        // );
    }
}
